import Foundation
import Observation

// MARK: - Sync Trigger & Store Registry

public enum FabushiSyncTrigger: Sendable, Equatable {
    case manual
    case initialAppear
    case wsInitialConnect
    case wsReconnect
    case pullToRefresh
}

/// Keeps a single shared `FabushiMessagingStore` per `FabushiCoordinator` instance using weak boxes
/// so isolated XCTest coordinators never leak or bleed into `.shared`.
@MainActor
public final class FabushiMessagingStoreRegistry {
    private final class WeakEntry {
        weak var coordinator: FabushiCoordinator?
        let store: FabushiMessagingStore

        init(coordinator: FabushiCoordinator, store: FabushiMessagingStore) {
            self.coordinator = coordinator
            self.store = store
        }
    }

    public static let shared = FabushiMessagingStoreRegistry()
    private var entries: [ObjectIdentifier: WeakEntry] = [:]

    private init() {}

    public func store(for coordinator: FabushiCoordinator) -> FabushiMessagingStore {
        pruneDeallocated()
        let key = ObjectIdentifier(coordinator)
        if let existing = entries[key], existing.coordinator === coordinator {
            return existing.store
        }
        let created = FabushiMessagingStore(coordinator: coordinator)
        entries[key] = WeakEntry(coordinator: coordinator, store: created)
        return created
    }

    public func removeStore(for coordinator: FabushiCoordinator) {
        let key = ObjectIdentifier(coordinator)
        if let entry = entries.removeValue(forKey: key) {
            entry.store.shutdown()
        }
    }

    private func pruneDeallocated() {
        let deadKeys = entries.compactMap { key, entry in
            entry.coordinator == nil ? key : nil
        }
        for key in deadKeys {
            entries.removeValue(forKey: key)?.store.shutdown()
        }
    }
}

// MARK: - Central M3 Messaging State Engine

@MainActor
@Observable
public final class FabushiMessagingStore {
    // MARK: Constants (`D13-04`, `D13-05`)
    public static let wsConnectDedupWindowSeconds: TimeInterval = 3.0
    public static let maxPrefetchDialogs: Int = 8
    public static let prefetchBatchSize: Int = 3
    public static let prefetchHistoryLimit: Int = 50
    public static let defaultPrefetchDelayNanoseconds: UInt64 = 250_000_000

    // MARK: Dependencies
    @ObservationIgnored public let coordinator: FabushiCoordinator
    @ObservationIgnored public let client: FabushiMessagingHostClient
    @ObservationIgnored public let cacheStore: FabushiMessagingCacheStore
    public let overlayController: FabushiMessagingOverlayController

    // MARK: Injectable Test Seams (Production Defaults to Real System APIs)
    @ObservationIgnored public var prefetchDelayNanoseconds: UInt64 = defaultPrefetchDelayNanoseconds
    @ObservationIgnored public var isLowPowerModeProvider: @MainActor () -> Bool = {
        ProcessInfo.processInfo.isLowPowerModeEnabled
    }
    @ObservationIgnored public var isConstrainedNetworkProvider: @MainActor () -> Bool

    // MARK: Observed Account Scope Tracking
    public private(set) var observedGeneration: UInt64 = 0
    public private(set) var observedAccountId: String? = nil

    // MARK: Dialogs, Folders, Topics & Selection (`#10`, `D13-01..05`, `D14-05`)
    public private(set) var chats: [FabushiMessagingChat] = []
    public private(set) var folders: [FabushiFolder] = []
    public private(set) var topicsByChat: [Int64: [FabushiForumTopic]] = [:]
    public var selectedBuiltInFolder: FabushiBuiltInFolderFilter = .all
    public var selectedCustomFolderId: Int64? = nil
    public var showArchivedSection: Bool = false
    public private(set) var activeChatId: Int64? = nil
    public private(set) var lastActiveChatId: Int64? = nil
    public private(set) var isLoadingDialogs: Bool = false
    public private(set) var hasSyncedDialogs: Bool = false
    public private(set) var hasHydratedFrame0: Bool = false
    public private(set) var offlinePreservedMode: Bool = false
    public private(set) var isBackgroundRevalidating: Bool = false
    public private(set) var lastDialogErrorMessage: String? = nil
    public private(set) var subscribedWsChatIds: Set<Int64> = []
    public private(set) var hasConnectedWsOnce: Bool = false

    // MARK: Messages, History, Pins, Typing & Bot Activity (`#12`, `#13`, `D13-03`, `D14-04`)
    public private(set) var messagesByChat: [Int64: [FabushiMessageRecord]] = [:]
    public private(set) var isLoadingHistoryByChat: [Int64: Bool] = [:]
    public private(set) var loadedHistoryChats: Set<Int64> = []
    public private(set) var pinnedMessagesByChat: [Int64: [FabushiPinnedMessageRecord]] = [:]
    public private(set) var ptsByChat: [Int64: UInt64] = [:]
    public private(set) var activeBotActivityByChat: [Int64: FabushiBotActivityState] = [:]
    public private(set) var typingUsersByChat: [Int64: [Int64: String]] = [:]
    public private(set) var knownBotIds: Set<Int64> = []
    public private(set) var stories: [FabushiStoryItem] = []

    // MARK: Sync & Outbox (`#12`)
    public private(set) var outboxEntries: [FabushiOutboxEntry] = []
    public private(set) var dismissedOutboxNonces: Set<UInt64> = []
    public private(set) var isRecoveringSyncGaps: Bool = false
    public private(set) var pendingOutboxCount: Int = 0

    // MARK: Drafts & Composer Support (`#15`, `D10-07`, `D14-03`)
    public private(set) var draftsByChat: [Int64: FabushiDraftRecord] = [:]
    public private(set) var unsentDraftTextByChat: [Int64: String] = [:]
    public private(set) var draftOpGenByChat: [Int64: UInt64] = [:]
    public private(set) var inFlightSendDraftGenByChat: [Int64: UInt64] = [:]
    public var replyTargetByChat: [Int64: FabushiReplyTarget] = [:]
    public private(set) var sendAsPeersByChat: [Int64: [FabushiSendAsPeer]] = [:]
    public var selectedSendAsPeerByChat: [Int64: UInt64] = [:]

    // MARK: Multi-Select Compose (`#9`, `D10-06`)
    public private(set) var isComposeActive: Bool = false
    public private(set) var composeEpoch: UInt64 = 0
    public private(set) var pendingCreationEpoch: UInt64? = nil
    public var composeQuery: String = ""
    public var composeGroupTitle: String = ""
    public private(set) var selectedComposeRecipients: [FabushiComposeRecipient] = []
    public private(set) var resolvedComposePeer: FabushiUserProfile? = nil
    public private(set) var isResolvingComposePeer: Bool = false
    public private(set) var isCreatingComposeChat: Bool = false
    public private(set) var composeErrorMessage: String? = nil

    // MARK: Contacts, Blocklist, Bots & Prestart Bot (`#11`, `D10-08`)
    public private(set) var contacts: [FabushiContact] = []
    public private(set) var blockedUsers: [FabushiBlockedUser] = []
    public private(set) var bots: [FabushiBotConfig] = []
    public private(set) var resolvedContactPeer: FabushiUserProfile? = nil
    public private(set) var isLoadingContacts: Bool = false
    public private(set) var botPublicProfilesByTarget: [String: FabushiBotPublicProfile] = [:]

    // MARK: Unified Search (`#18`, `BLK-08`)
    public var searchQuery: String = ""
    public var searchScope: FabushiSearchScope = .all
    public private(set) var searchResults: [FabushiSearchResult] = []
    public private(set) var isSearching: Bool = false
    public private(set) var isUsingOfflineSearchFallback: Bool = false

    // MARK: M3b Stage Actions, Slash Commands (`D11-01`), Approvals, Floor, E2EE & Media (`#13`..`#20`)
    public private(set) var slashCommands: [FabushiSlashCommand] = []
    public private(set) var distilledScriptsByChat: [Int64: [FabushiDistilledBrowserScript]] = [:]
    public private(set) var pendingApprovals: [FabushiApprovalItem] = []
    public private(set) var floorSpeakerByChat: [Int64: String] = [:]
    public private(set) var verifiedE2EEChatIds: Set<Int64> = []
    public private(set) var e2eeSafetyByChat: [Int64: FabushiE2EESafetyInfo] = [:]
    public var editingMessageByChat: [Int64: FabushiMessageRecord] = [:]
    public var activeTopicIdByChat: [Int64: Int64] = [:]
    public var botReplyKeyboardByChat: [Int64: [[String]]] = [:]
    public private(set) var lastBrowserTakeoverAction: String? = nil

    // MARK: Private Runtime State
    @ObservationIgnored private var eventLoopTask: Task<Void, Never>?
    @ObservationIgnored private var prefetchTask: Task<Void, Never>?
    @ObservationIgnored private var searchTask: Task<Void, Never>?
    @ObservationIgnored private var nonceCounter: UInt64 = 0
    @ObservationIgnored private var provisionalIdCounter: Int64 = -1
    @ObservationIgnored private var chatListInFlightSince: ContinuousClock.Instant? = nil
    @ObservationIgnored private var lastChatListCompletedAt: ContinuousClock.Instant? = nil
    @ObservationIgnored private var historyInFlightChats: Set<Int64> = []
    @ObservationIgnored public private(set) var wsRefreshDispatchCount: Int = 0
    @ObservationIgnored public private(set) var prefetchExecutedChatIds: [Int64] = []

    public init(
        coordinator: FabushiCoordinator,
        cacheStore: FabushiMessagingCacheStore = FabushiMessagingCacheStore(),
        overlayController: FabushiMessagingOverlayController = FabushiMessagingOverlayController()
    ) {
        self.coordinator = coordinator
        self.client = FabushiMessagingHostClient(coordinator: coordinator)
        self.cacheStore = cacheStore
        self.overlayController = overlayController
        self.isConstrainedNetworkProvider = { [weak coordinator] in
            coordinator?.networkMonitor.currentSnapshot?.isConstrained ?? false
        }
        self.observedGeneration = coordinator.accountScopeGeneration
        self.observedAccountId = coordinator.activeAccountId

        registerPurgers()
    }

    public func shutdown() {
        eventLoopTask?.cancel()
        eventLoopTask = nil
        prefetchTask?.cancel()
        prefetchTask = nil
        searchTask?.cancel()
        searchTask = nil
        coordinator.unregisterAccountSecretsPurger(id: FabushiMessagingCacheStore.accountPurgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: FabushiMessagingCacheStore.serverSwitchPurgerId)
    }

    private func registerPurgers() {
        let storeRef = cacheStore
        coordinator.registerAccountSecretsPurger(id: FabushiMessagingCacheStore.accountPurgerId) { [weak self] departingAccountId in
            storeRef.purgeAccount(accountId: departingAccountId)
            self?.resetAllInMemoryState(forGeneration: self?.coordinator.accountScopeGeneration ?? 0, accountId: nil)
        }
        coordinator.registerServerSwitchSecretsPurger(id: FabushiMessagingCacheStore.serverSwitchPurgerId) { [weak self] in
            storeRef.purgeAll()
            self?.resetAllInMemoryState(forGeneration: self?.coordinator.accountScopeGeneration ?? 0, accountId: nil)
        }
    }

    // MARK: - Account Scope Reconciliation & Full Memory Reset

    @discardableResult
    public func reconcileAccountScopeIfNeeded() -> Bool {
        let currentGen = coordinator.accountScopeGeneration
        let currentAccount = coordinator.activeAccountId
        guard currentGen != observedGeneration || currentAccount != observedAccountId else {
            return false
        }
        resetAllInMemoryState(forGeneration: currentGen, accountId: currentAccount)
        return true
    }

    public func resetAllInMemoryState(forGeneration generation: UInt64, accountId: String?) {
        prefetchTask?.cancel()
        prefetchTask = nil
        searchTask?.cancel()
        searchTask = nil

        observedGeneration = generation
        observedAccountId = accountId

        chats.removeAll()
        folders.removeAll()
        topicsByChat.removeAll()
        selectedBuiltInFolder = .all
        selectedCustomFolderId = nil
        showArchivedSection = false
        activeChatId = nil
        lastActiveChatId = nil
        isLoadingDialogs = false
        hasSyncedDialogs = false
        hasHydratedFrame0 = false
        offlinePreservedMode = false
        isBackgroundRevalidating = false
        lastDialogErrorMessage = nil
        subscribedWsChatIds.removeAll()
        hasConnectedWsOnce = false
        chatListInFlightSince = nil
        lastChatListCompletedAt = nil
        historyInFlightChats.removeAll()

        messagesByChat.removeAll()
        isLoadingHistoryByChat.removeAll()
        loadedHistoryChats.removeAll()
        pinnedMessagesByChat.removeAll()
        ptsByChat.removeAll()
        activeBotActivityByChat.removeAll()
        typingUsersByChat.removeAll()
        knownBotIds.removeAll()
        stories.removeAll()

        outboxEntries.removeAll()
        dismissedOutboxNonces.removeAll()
        isRecoveringSyncGaps = false
        pendingOutboxCount = 0

        draftsByChat.removeAll()
        unsentDraftTextByChat.removeAll()
        draftOpGenByChat.removeAll()
        inFlightSendDraftGenByChat.removeAll()
        replyTargetByChat.removeAll()
        sendAsPeersByChat.removeAll()
        selectedSendAsPeerByChat.removeAll()

        cancelComposeMode()

        contacts.removeAll()
        blockedUsers.removeAll()
        bots.removeAll()
        resolvedContactPeer = nil
        isLoadingContacts = false
        botPublicProfilesByTarget.removeAll()

        searchQuery = ""
        searchScope = .all
        searchResults.removeAll()
        isSearching = false
        isUsingOfflineSearchFallback = false

        slashCommands.removeAll()
        distilledScriptsByChat.removeAll()
        pendingApprovals.removeAll()
        floorSpeakerByChat.removeAll()
        verifiedE2EEChatIds.removeAll()
        e2eeSafetyByChat.removeAll()
        editingMessageByChat.removeAll()
        activeTopicIdByChat.removeAll()
        botReplyKeyboardByChat.removeAll()
        lastBrowserTakeoverAction = nil

        overlayController.dismissAll()
    }

    // MARK: - Live Event Stream Consumer (TRAP #2: `rawPayloadJSON` + Generation Fence)

    public func startEventLoopIfNeeded() {
        guard eventLoopTask == nil else { return }
        let stream = coordinator.host.makeEventStream()
        eventLoopTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard !Task.isCancelled, let self else { break }
                _ = self.ingestDomainEvent(event)
            }
        }
    }

    /// Processes a single host/WS domain event strictly fenced by `coordinator.accountScopeGeneration`.
    /// Decodes `event.rawPayloadJSON` directly for `newMessage`, `editMessage`, and `reactionUpdated`
    /// so nested Rust `MessageItem` and `is_chosen_by_me` fields are never lost (TRAP #2).
    @discardableResult
    public func ingestDomainEvent(_ event: FabushiDomainEvent) -> Bool {
        _ = reconcileAccountScopeIfNeeded()
        guard event.generation >= coordinator.accountScopeGeneration else {
            return false
        }

        switch event.type {
        case "newMessage":
            if var decoded = FabushiMessageRecord.decodeFromDomainEventPayload(event.rawPayloadJSON) {
                if case .authenticated(let myUserId, _, _) = coordinator.authState,
                   decoded.senderId == myUserId {
                    decoded.isOutgoing = true
                }
                applyIncomingMessage(decoded)
                return true
            }
            if case .newMessage(let id, let chatId, let senderId, let text, let timestamp, let replyTo, let clientNonce, let status) = event.kind {
                let fallback = FabushiMessageRecord(
                    messageId: id,
                    chatId: chatId,
                    senderId: senderId >= 0 ? UInt64(senderId) : 0,
                    text: text,
                    dateMs: FabushiWireDateParser.normalizeEpochToMs(timestamp),
                    isOutgoing: isCurrentUser(senderId),
                    replyTo: replyTo.map { FabushiReplyHeader(replyToMsgId: $0) },
                    clientNonce: clientNonce,
                    deliveryStatus: FabushiMessageDeliveryStatus(wireStatus: status)
                )
                applyIncomingMessage(fallback)
                return true
            }
            return false

        case "editMessage":
            if let decoded = FabushiMessageRecord.decodeFromDomainEventPayload(event.rawPayloadJSON) {
                applyEditedMessage(
                    chatId: decoded.chatId,
                    messageId: decoded.messageId,
                    text: decoded.text,
                    entities: decoded.entities,
                    editDateMs: decoded.editDateMs ?? event.timestampMs
                )
                return true
            }
            if case .editMessage(let id, let chatId, let text, let editTs) = event.kind {
                applyEditedMessage(
                    chatId: chatId,
                    messageId: id,
                    text: text,
                    entities: [],
                    editDateMs: FabushiWireDateParser.normalizeEpochToMs(editTs)
                )
                return true
            }
            return false

        case "deleteMessages":
            if case .deleteMessages(let chatId, let messageIds, let forEveryone) = event.kind {
                applyDeletedMessages(chatId: chatId, messageIds: messageIds, forEveryone: forEveryone)
                return true
            }
            return false

        case "reactionUpdated":
            if let parsed = FabushiMessageRecord.decodeReactionsFromEventPayload(event.rawPayloadJSON) {
                applyReactionsUpdate(chatId: parsed.chatId, messageId: parsed.messageId, reactions: parsed.reactions)
                return true
            }
            if case .reactionUpdated(let chatId, let messageId, _, let reactions) = event.kind {
                let mapped = reactions.map {
                    FabushiMessageReaction(emoji: $0.emoji, count: $0.count, isChosenByMe: $0.chosenByMe)
                }
                applyReactionsUpdate(chatId: chatId, messageId: messageId, reactions: mapped)
                return true
            }
            return false

        case "pollUpdated":
            if case .pollUpdated(let chatId, let messageId, let payload) = event.kind,
               let data = try? JSONEncoder().encode(payload),
               let poll = try? JSONDecoder().decode(FabushiPollCard.self, from: data) {
                updateMessage(chatId: chatId, messageId: messageId) { msg in
                    msg.content = .poll(poll)
                }
                return true
            }
            return false

        case "todoUpdated":
            if case .todoUpdated(let chatId, let messageId, let payload) = event.kind,
               let data = try? JSONEncoder().encode(payload),
               let todo = try? JSONDecoder().decode(FabushiTodoCard.self, from: data) {
                updateMessage(chatId: chatId, messageId: messageId) { msg in
                    msg.content = .todo(todo)
                }
                return true
            }
            return false

        case "readHistory":
            if case .readHistory(let chatId, let maxId, let isOutbox, let unreadCount) = event.kind {
                if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                    if isOutbox {
                        chats[idx].readOutboxMaxId = max(chats[idx].readOutboxMaxId, maxId)
                        if var msgs = messagesByChat[chatId] {
                            for i in msgs.indices where msgs[i].isOutgoing && msgs[i].messageId > 0 && msgs[i].messageId <= maxId {
                                msgs[i].deliveryStatus = .read
                            }
                            messagesByChat[chatId] = msgs
                        }
                    } else {
                        chats[idx].readInboxMaxId = max(chats[idx].readInboxMaxId, maxId)
                        chats[idx].unreadCount = max(0, unreadCount)
                    }
                    persistSnapshotToDisk()
                }
                return true
            }
            return false

        case "userTyping":
            if case .userTyping(let chatId, let userId, let action) = event.kind {
                var map = typingUsersByChat[chatId] ?? [:]
                if action.lowercased() == "cancel" {
                    map.removeValue(forKey: userId)
                } else {
                    map[userId] = action
                }
                typingUsersByChat[chatId] = map.isEmpty ? nil : map
                return true
            }
            return false

        case "pinnedMessagesChanged":
            if case .pinnedMessagesChanged(let chatId, let pinnedIds) = event.kind {
                if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                    chats[idx].pinnedMessageId = pinnedIds.first
                }
                let existing = pinnedMessagesByChat[chatId] ?? []
                let byId = Dictionary(uniqueKeysWithValues: existing.map { ($0.messageId, $0) })
                pinnedMessagesByChat[chatId] = pinnedIds.map { pid in
                    byId[pid] ?? FabushiPinnedMessageRecord(chatId: chatId, messageId: pid)
                }
                return true
            }
            return false

        case "draftUpdated":
            if case .draftUpdated(let chatId, _, let payload) = event.kind {
                if case .null = payload {
                    applyRemoteDraft(chatId: chatId, draft: nil)
                } else if let data = try? JSONEncoder().encode(payload),
                          let draft = try? JSONDecoder().decode(FabushiDraftRecord.self, from: data) {
                    applyRemoteDraft(chatId: chatId, draft: draft)
                }
                return true
            }
            return false

        case "folderChanged":
            if case .folderChanged(let payload) = event.kind,
               let data = try? JSONEncoder().encode(payload),
               let folder = try? JSONDecoder().decode(FabushiFolder.self, from: data) {
                upsertFolder(folder)
                return true
            }
            return false

        case "forumTopicChanged":
            if case .forumTopicChanged(let chatId, let payload) = event.kind,
               let data = try? JSONEncoder().encode(payload),
               let topic = try? JSONDecoder().decode(FabushiForumTopic.self, from: data) {
                upsertTopic(topic, inChatId: chatId)
                return true
            }
            return false

        case "storyUpdated":
            if case .storyUpdated(let payload) = event.kind,
               let data = try? JSONEncoder().encode(payload),
               let story = try? JSONDecoder().decode(FabushiStoryItem.self, from: data) {
                upsertStory(story)
                return true
            }
            return false

        case "botActivityChanged":
            if case .botActivityChanged(let chatId, let botUserId, let state, let detail) = event.kind {
                applyBotActivityEvent(chatId: chatId, botUserId: botUserId, state: state, detail: detail)
                return true
            }
            return false

        case "messageAck":
            if case .messageAck(_, let clientNonce, let messageId, let pts) = event.kind {
                if let clientNonce {
                    reconcileProvisionalMessage(clientNonce: clientNonce, serverMessageId: messageId, status: .sent)
                }
                if let pts, let chatId = activeChatId {
                    ptsByChat[chatId] = max(ptsByChat[chatId] ?? 0, pts)
                }
                return true
            }
            return false

        case "syncGapDetected":
            if case .syncGapDetected(let chatId, _) = event.kind {
                Task { @MainActor [weak self] in
                    await self?.syncDifferenceForChat(chatId)
                }
                return true
            }
            return false

        case "syncDifference":
            if case .syncDifference(let chatId, let pts, _, _) = event.kind, let chatId {
                ptsByChat[chatId] = max(ptsByChat[chatId] ?? 0, pts)
                return true
            }
            return false

        case "outboxStatusChanged":
            if case .outboxStatusChanged(let clientNonce, let chatId, let statusStr, let serverMessageId, let retryCount, let lastError) = event.kind {
                applyOutboxStatusChanged(
                    clientNonce: clientNonce,
                    chatId: chatId,
                    status: FabushiOutboxStatus(wireValue: statusStr),
                    serverMessageId: serverMessageId,
                    retryCount: retryCount,
                    lastError: lastError
                )
                return true
            }
            return false

        case "connectionStateChanged":
            if case .connectionStateChanged(let status, _) = event.kind {
                handleConnectionStateChange(status: status)
                return true
            }
            return false

        case "agentFloorChanged":
            if case .agentFloorChanged(let chatId, let currentSpeaker, _, _) = event.kind {
                if let speaker = currentSpeaker?.trimmingCharacters(in: .whitespacesAndNewlines), !speaker.isEmpty {
                    floorSpeakerByChat[chatId] = speaker
                } else {
                    floorSpeakerByChat.removeValue(forKey: chatId)
                }
                return true
            }
            return false

        case "toolApprovalRequested":
            if case .toolApprovalRequested(let approvalId, let toolName, let summary) = event.kind {
                let item = FabushiApprovalItem(
                    id: approvalId,
                    sessionId: nil,
                    command: summary.isEmpty ? toolName : "\(toolName): \(summary)",
                    riskLevel: "high",
                    patternName: toolName,
                    status: "pending"
                )
                pendingApprovals.removeAll { $0.id == approvalId }
                pendingApprovals.insert(item, at: 0)
                return true
            }
            return false

        default:
            return false
        }
    }

    // MARK: - `D14-04` Bot Activity & Non-Corruption Invariant

    /// `D14-04`: Both 1:1 bot DMs AND group chats can display live bot activity,
    /// without EVER corrupting `chat.isBot` or inserting group IDs into `knownBotIds`.
    public func canDisplayBotActivity(for chatId: Int64) -> Bool {
        guard let chat = chats.first(where: { $0.chatId == chatId }) else { return false }
        return chat.isBot || chat.isGroup
    }

    public func applyBotActivityEvent(chatId: Int64, botUserId: Int64, state: String, detail: String?) {
        let activity = FabushiBotActivityState(
            chatId: chatId,
            botUserId: botUserId,
            state: state,
            detail: detail
        )
        if activity.isActive {
            activeBotActivityByChat[chatId] = activity
        } else {
            activeBotActivityByChat.removeValue(forKey: chatId)
        }
        // Strictly DO NOT flip `chat.isBot = true` or add `chatId` into `knownBotIds`!
        // Only register `botUserId` if it is a positive distinct bot peer, never the group's chatId.
        if let chat = chats.first(where: { $0.chatId == chatId }),
           chat.isBot && !chat.isGroup && !chat.isChannel {
            knownBotIds.insert(chatId)
        }
    }

    // MARK: - Frame-0 Hydration & Parallel Workspace Sync (`D13-01..D13-05`)

    /// Synchronous Frame-0 cache hydration BEFORE network completes (`D13-01`).
    @discardableResult
    public func hydrateFrame0() -> Bool {
        _ = reconcileAccountScopeIfNeeded()
        guard let accountId = coordinator.activeAccountId, !accountId.isEmpty else {
            return false
        }
        let serverURL = coordinator.settingsStore.activeServerURL
        guard let snapshot = cacheStore.loadSnapshot(serverURL: serverURL, accountId: accountId) else {
            return false
        }

        chats = sortChatsDeterministically(snapshot.chats)
        folders = snapshot.folders.sorted { $0.sortOrder < $1.sortOrder }
        contacts = snapshot.contacts
        messagesByChat = snapshot.messagesByChat
        draftsByChat = snapshot.draftsByChat
        ptsByChat = snapshot.ptsByChat
        lastActiveChatId = snapshot.lastActiveChatId
        for (chatId, draft) in snapshot.draftsByChat where !draft.text.isEmpty {
            unsentDraftTextByChat[chatId] = draft.text
        }
        rebuildKnownBotIds()
        hasHydratedFrame0 = true
        return true
    }

    /// Refreshes dialogs, folders, outbox, contacts & bots in parallel via `async let` (`D13-04`),
    /// honoring `<3s` WS initial-connect deduplication, `D13-02` 401/403 vs transient offline rules,
    /// and scheduling the `D13-05` bounded top-8 background history prefetcher.
    public func refreshWorkspace(trigger: FabushiSyncTrigger = .manual) async {
        _ = reconcileAccountScopeIfNeeded()
        guard coordinator.isStarted else { return }

        let now = ContinuousClock.now
        if trigger == .wsInitialConnect {
            if let inFlight = chatListInFlightSince,
               now - inFlight < .seconds(Self.wsConnectDedupWindowSeconds) {
                return
            }
            if let completed = lastChatListCompletedAt,
               now - completed < .seconds(Self.wsConnectDedupWindowSeconds) {
                return
            }
        }

        let capturedGen = coordinator.accountScopeGeneration
        let capturedAccount = coordinator.activeAccountId
        let serverURL = coordinator.settingsStore.activeServerURL

        if chats.isEmpty {
            isLoadingDialogs = true
        } else {
            isBackgroundRevalidating = true
        }
        chatListInFlightSince = now
        wsRefreshDispatchCount += 1
        lastDialogErrorMessage = nil

        defer {
            if coordinator.accountScopeGeneration == capturedGen {
                isLoadingDialogs = false
                isBackgroundRevalidating = false
                chatListInFlightSince = nil
                lastChatListCompletedAt = .now
            }
        }

        do {
            async let chatsTask = client.listChats(localOnly: false, fallbackLocal: false)
            async let foldersTask: [FabushiFolder]? = try? await client.listFolders()
            async let outboxTask: [FabushiOutboxEntry]? = try? await client.listOutbox()
            async let contactsTask: [FabushiContact]? = try? await client.listContacts()
            async let botsTask: [FabushiBotConfig]? = try? await client.listBots()

            let fetchedChats = try await chatsTask
            let (fetchedFolders, fetchedOutbox, fetchedContacts, fetchedBots) = await (
                foldersTask,
                outboxTask,
                contactsTask,
                botsTask
            )

            guard coordinator.accountScopeGeneration == capturedGen,
                  coordinator.activeAccountId == capturedAccount
            else {
                return
            }

            let existingById = Dictionary(uniqueKeysWithValues: chats.map { ($0.chatId, $0) })
            let mergedChats: [FabushiMessagingChat] = fetchedChats.map { incoming in
                if let existing = existingById[incoming.chatId] {
                    return existing.mergedPreservingRichLocalFlags(from: incoming)
                }
                return incoming
            }

            chats = sortChatsDeterministically(mergedChats)
            if let fetchedFolders {
                folders = fetchedFolders.sorted { $0.sortOrder < $1.sortOrder }
            }
            if let fetchedOutbox {
                applyOutboxList(fetchedOutbox)
            }
            if let fetchedContacts {
                contacts = fetchedContacts
            }
            if let fetchedBots {
                bots = fetchedBots
            }

            rebuildKnownBotIds()
            hasSyncedDialogs = true
            offlinePreservedMode = false
            persistSnapshotToDisk(serverURL: serverURL, accountId: capturedAccount)
            scheduleBackgroundHistoryPrefetch(capturedGeneration: capturedGen)
        } catch let error as FabushiCoordinatorError {
            if case .staleGeneration = error { return }
        } catch let hostError as FabushiHostError {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            if hostError.isCancelled || hostError.code == "stale_generation" {
                return
            }
            // `D13-02`: 401/403 wipes cached snapshots cleanly; transient network/5xx preserves offline cache!
            switch hostError {
            case .unauthorized, .permissionDenied:
                if let capturedAccount {
                    cacheStore.purgeAccount(accountId: capturedAccount)
                }
                resetAllInMemoryState(forGeneration: capturedGen, accountId: nil)
                lastDialogErrorMessage = hostError.localizedDescription
            case .networkError, .timeout, .rateLimited:
                offlinePreservedMode = true
                lastDialogErrorMessage = hostError.localizedDescription
                await hydrateLocalStoreFallbackIfEmpty(capturedGeneration: capturedGen)
            default:
                if hostError.isRetryable {
                    offlinePreservedMode = true
                    await hydrateLocalStoreFallbackIfEmpty(capturedGeneration: capturedGen)
                }
                lastDialogErrorMessage = hostError.localizedDescription
            }
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            if error is CancellationError { return }
            offlinePreservedMode = true
            lastDialogErrorMessage = error.localizedDescription
            await hydrateLocalStoreFallbackIfEmpty(capturedGeneration: capturedGen)
        }
    }

    private func hydrateLocalStoreFallbackIfEmpty(capturedGeneration: UInt64) async {
        guard chats.isEmpty else { return }
        if let localChats = try? await client.listChats(localOnly: true),
           coordinator.accountScopeGeneration == capturedGeneration,
           !localChats.isEmpty {
            chats = sortChatsDeterministically(localChats)
            rebuildKnownBotIds()
        }
        if let localOutbox = try? await client.listOutbox(),
           coordinator.accountScopeGeneration == capturedGeneration {
            applyOutboxList(localOutbox)
        }
    }

    /// `D13-05`: 250ms delayed, bounded top-8 (pinned first), batch-3 background history prefetcher
    /// respecting `accountScopeGeneration`, Low Power Mode, constrained network (Low Data Mode), and online state.
    public func scheduleBackgroundHistoryPrefetch(capturedGeneration: UInt64) {
        prefetchTask?.cancel()
        guard coordinator.isOnline else { return }
        guard !isLowPowerModeProvider() else { return }
        guard !isConstrainedNetworkProvider() else { return }

        let delayNs = prefetchDelayNanoseconds
        prefetchTask = Task { @MainActor [weak self] in
            if delayNs > 0 {
                try? await Task.sleep(nanoseconds: delayNs)
            }
            guard !Task.isCancelled,
                  let self,
                  self.coordinator.isStarted,
                  self.coordinator.accountScopeGeneration == capturedGeneration,
                  self.coordinator.isOnline,
                  !self.isLowPowerModeProvider(),
                  !self.isConstrainedNetworkProvider()
            else {
                return
            }

            let candidates = self.selectPrefetchCandidates()
            guard !candidates.isEmpty else { return }

            var offset = 0
            while offset < candidates.count {
                guard !Task.isCancelled,
                      self.coordinator.accountScopeGeneration == capturedGeneration,
                      self.coordinator.isOnline,
                      !self.isLowPowerModeProvider(),
                      !self.isConstrainedNetworkProvider()
                else {
                    break
                }

                let end = min(offset + Self.prefetchBatchSize, candidates.count)
                let batch = Array(candidates[offset..<end])
                offset = end

                struct PrefetchHistoryParams: Encodable, Sendable {
                    let chat_id: UInt64
                    let limit: Int
                }
                let host = self.coordinator.host
                let capturedAccount = self.coordinator.activeAccountId
                let limit = Self.prefetchHistoryLimit

                await withTaskGroup(of: (Int64, [FabushiMessageRecord]?).self) { group in
                    for chatId in batch where chatId >= 0 {
                        group.addTask {
                            let fetched: [FabushiMessageRecord]? = try? await host.dispatch(
                                method: FabushiRPCNamespace.messages.method("history"),
                                params: PrefetchHistoryParams(chat_id: UInt64(chatId), limit: limit),
                                generation: capturedGeneration,
                                accountId: capturedAccount
                            )
                            return (chatId, fetched)
                        }
                    }

                    for await (chatId, fetched) in group {
                        guard self.coordinator.accountScopeGeneration == capturedGeneration else { return }
                        if let fetched {
                            self.prefetchExecutedChatIds.append(chatId)
                            self.mergeFetchedHistory(fetched, forChatId: chatId)
                        }
                    }
                }
            }

            if self.coordinator.accountScopeGeneration == capturedGeneration {
                self.persistSnapshotToDisk()
            }
        }
    }

    public func selectPrefetchCandidates() -> [Int64] {
        let sorted = sortChatsDeterministically(chats.filter { !$0.isArchived })
        var selected: [Int64] = []
        selected.reserveCapacity(Self.maxPrefetchDialogs)
        for chat in sorted {
            if chat.chatId == activeChatId { continue }
            if loadedHistoryChats.contains(chat.chatId) { continue }
            if let cached = messagesByChat[chat.chatId], !cached.isEmpty { continue }
            selected.append(chat.chatId)
            if selected.count >= Self.maxPrefetchDialogs {
                break
            }
        }
        return selected
    }

    private func handleConnectionStateChange(status: String) {
        let lower = status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard lower == "connected" || lower == "online" else { return }
        let isReconnect = hasConnectedWsOnce
        hasConnectedWsOnce = true

        if isReconnect {
            subscribedWsChatIds.removeAll()
            if let activeChatId {
                Task { @MainActor [weak self] in
                    await self?.subscribeWsIfNeeded(chatId: activeChatId, force: true)
                }
            }
            Task { @MainActor [weak self] in
                await self?.refreshWorkspace(trigger: .wsReconnect)
            }
        } else {
            Task { @MainActor [weak self] in
                await self?.refreshWorkspace(trigger: .wsInitialConnect)
            }
        }
    }

    // MARK: - Chat Selection, Immediate WS Subscribe (`D14-05`) & Parallel History Load (`D13-03`/`D13-04`)

    public func selectChat(_ chatId: Int64?) async {
        _ = reconcileAccountScopeIfNeeded()
        if isComposeActive && chatId != nil {
            cancelComposeMode()
        }
        activeChatId = chatId
        guard let chatId else { return }

        lastActiveChatId = chatId
        let hasCachedMessages = !(messagesByChat[chatId]?.isEmpty ?? true)
        if !hasCachedMessages && !loadedHistoryChats.contains(chatId) {
            isLoadingHistoryByChat[chatId] = true
        }

        await subscribeWsIfNeeded(chatId: chatId, force: false)
        await loadChatStageWave(chatId: chatId)
    }

    @discardableResult
    public func subscribeWsIfNeeded(chatId: Int64, force: Bool = false) async -> Bool {
        guard force || !subscribedWsChatIds.contains(chatId) else { return true }
        let capturedGen = coordinator.accountScopeGeneration
        do {
            _ = try await client.subscribeWs(chatId: chatId)
            if coordinator.accountScopeGeneration == capturedGen {
                subscribedWsChatIds.insert(chatId)
            }
            return true
        } catch {
            return false
        }
    }

    /// Parallel Wave (`D13-04`): fetches history + pins + draft + send-as peers concurrently.
    public func loadChatStageWave(chatId: Int64) async {
        guard !historyInFlightChats.contains(chatId) else { return }
        let capturedGen = coordinator.accountScopeGeneration
        historyInFlightChats.insert(chatId)
        if messagesByChat[chatId]?.isEmpty ?? true {
            isLoadingHistoryByChat[chatId] = true
        }

        defer {
            if coordinator.accountScopeGeneration == capturedGen {
                historyInFlightChats.remove(chatId)
                isLoadingHistoryByChat[chatId] = false
            }
        }

        let draftGen = nextDraftOpGen(for: chatId)

        async let historyTask: [FabushiMessageRecord]? = try? await client.fetchHistory(chatId: chatId, limit: Self.prefetchHistoryLimit)
        async let pinsTask: [FabushiPinnedMessageRecord]? = try? await client.listPinnedMessages(chatId: chatId)
        async let draftTask: FabushiDraftRecord? = try? await client.getDraft(chatId: chatId)
        async let sendAsTask: [FabushiSendAsPeer]? = try? await client.listSendAsPeers(chatId: chatId)

        let (fetchedHistory, fetchedPins, fetchedDraft, fetchedSendAs) = await (
            historyTask,
            pinsTask,
            draftTask,
            sendAsTask
        )

        guard coordinator.accountScopeGeneration == capturedGen else { return }

        if let fetchedHistory {
            mergeFetchedHistory(fetchedHistory, forChatId: chatId)
        } else if let localFallback = try? await client.listLocalMessages(chatId: chatId),
                  coordinator.accountScopeGeneration == capturedGen {
            mergeFetchedHistory(localFallback, forChatId: chatId)
        }

        if let fetchedPins {
            pinnedMessagesByChat[chatId] = fetchedPins
        }

        // `D10-07`: Drop stale `draftGet` completion if user typed/modified draft while in flight.
        if draftOpGenByChat[chatId] == draftGen {
            applyRemoteDraft(chatId: chatId, draft: fetchedDraft)
        }

        if let fetchedSendAs {
            sendAsPeersByChat[chatId] = fetchedSendAs
            if selectedSendAsPeerByChat[chatId] == nil,
               let defaultPeer = fetchedSendAs.first(where: \.isDefault) ?? fetchedSendAs.first {
                selectedSendAsPeerByChat[chatId] = defaultPeer.peerId
            }
        }

        persistSnapshotToDisk()
    }

    // MARK: - Chat Mutations (Pin Stability, Archive, Mute, Mark Read, Delete, TTL `D10-08`)

    public func filteredChats(searchText: String = "") -> [FabushiMessagingChat] {
        let base: [FabushiMessagingChat]
        if showArchivedSection {
            base = chats.filter(\.isArchived)
        } else if let customId = selectedCustomFolderId,
                  let folder = folders.first(where: { $0.id == customId }) {
            base = chats.filter { folder.matches($0) }
        } else {
            let nonArchived = chats.filter { !$0.isArchived }
            switch selectedBuiltInFolder {
            case .all:
                base = nonArchived
            case .unread:
                base = nonArchived.filter { $0.unreadCount > 0 }
            case .bots:
                base = nonArchived.filter { $0.isBot && !$0.isGroup && !$0.isChannel }
            }
        }

        let trimmed = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let filtered: [FabushiMessagingChat]
        if trimmed.isEmpty {
            filtered = base
        } else {
            filtered = base.filter { chat in
                chat.title.lowercased().contains(trimmed)
                    || (chat.username?.lowercased().contains(trimmed) ?? false)
                    || (chat.lastMessagePreview?.lowercased().contains(trimmed) ?? false)
            }
        }
        return sortChatsDeterministically(filtered)
    }

    public var archivedChatsCount: Int {
        chats.filter(\.isArchived).count
    }

    public func sortChatsDeterministically(_ items: [FabushiMessagingChat]) -> [FabushiMessagingChat] {
        items.sorted { lhs, rhs in
            if lhs.isPinned != rhs.isPinned {
                return lhs.isPinned && !rhs.isPinned
            }
            if lhs.lastMessageDateMs != rhs.lastMessageDateMs {
                return lhs.lastMessageDateMs > rhs.lastMessageDateMs
            }
            return lhs.chatId < rhs.chatId
        }
    }

    public func togglePin(chatId: Int64) async {
        guard let idx = chats.firstIndex(where: { $0.chatId == chatId }) else { return }
        let targetPinned = !chats[idx].isPinned
        let previous = chats[idx].isPinned
        let capturedGen = coordinator.accountScopeGeneration
        chats[idx].isPinned = targetPinned
        chats = sortChatsDeterministically(chats)

        do {
            try await client.setChatPinned(chatId: chatId, pinned: targetPinned)
            if coordinator.accountScopeGeneration == capturedGen {
                persistSnapshotToDisk()
            }
        } catch {
            if coordinator.accountScopeGeneration == capturedGen,
               let rollbackIdx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[rollbackIdx].isPinned = previous
                chats = sortChatsDeterministically(chats)
            }
        }
    }

    public func toggleArchive(chatId: Int64) async {
        guard let idx = chats.firstIndex(where: { $0.chatId == chatId }) else { return }
        let targetArchived = !chats[idx].isArchived
        let previous = chats[idx].isArchived
        let capturedGen = coordinator.accountScopeGeneration
        chats[idx].isArchived = targetArchived

        do {
            try await client.setChatArchived(chatId: chatId, archived: targetArchived)
            if coordinator.accountScopeGeneration == capturedGen {
                persistSnapshotToDisk()
            }
        } catch {
            if coordinator.accountScopeGeneration == capturedGen,
               let rollbackIdx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[rollbackIdx].isArchived = previous
            }
        }
    }

    public func toggleMute(chatId: Int64, muteUntilMs: Int64? = nil) async {
        guard let idx = chats.firstIndex(where: { $0.chatId == chatId }) else { return }
        let targetMuted = !chats[idx].isMuted
        let previousMuted = chats[idx].isMuted
        let previousUntil = chats[idx].muteUntilMs
        let capturedGen = coordinator.accountScopeGeneration
        chats[idx].isMuted = targetMuted
        chats[idx].muteUntilMs = targetMuted ? muteUntilMs : nil

        do {
            try await client.setChatMuted(chatId: chatId, muted: targetMuted, muteUntilMs: muteUntilMs)
            if coordinator.accountScopeGeneration == capturedGen {
                persistSnapshotToDisk()
            }
        } catch {
            if coordinator.accountScopeGeneration == capturedGen,
               let rollbackIdx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[rollbackIdx].isMuted = previousMuted
                chats[rollbackIdx].muteUntilMs = previousUntil
            }
        }
    }

    public func markChatRead(chatId: Int64) async {
        guard let idx = chats.firstIndex(where: { $0.chatId == chatId }) else { return }
        let maxMsgId = max(
            chats[idx].lastMessageId,
            messagesByChat[chatId]?.map(\.messageId).max() ?? 0
        )
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let remaining = try await client.markRead(chatId: chatId, maxId: maxMsgId)
            if coordinator.accountScopeGeneration == capturedGen,
               let updatedIdx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[updatedIdx].unreadCount = max(0, remaining)
                chats[updatedIdx].readInboxMaxId = max(chats[updatedIdx].readInboxMaxId, maxMsgId)
                persistSnapshotToDisk()
            }
        } catch {
            // Keep server-authoritative unread count on failure.
        }
    }

    /// `D10-06`: Deleting an unrelated chat strictly preserves active compose state, recipients, and group title!
    public func deleteChat(chatId: Int64) async throws {
        let capturedGen = coordinator.accountScopeGeneration
        try await client.deleteChat(chatId: chatId)
        guard coordinator.accountScopeGeneration == capturedGen else { return }
        chats.removeAll { $0.chatId == chatId }
        messagesByChat.removeValue(forKey: chatId)
        loadedHistoryChats.remove(chatId)
        draftsByChat.removeValue(forKey: chatId)
        unsentDraftTextByChat.removeValue(forKey: chatId)
        pinnedMessagesByChat.removeValue(forKey: chatId)
        activeBotActivityByChat.removeValue(forKey: chatId)
        subscribedWsChatIds.remove(chatId)
        if activeChatId == chatId {
            activeChatId = nil
        }
        rebuildKnownBotIds()
        persistSnapshotToDisk()
    }

    /// `D10-08`: Emits TTL confirmation toast ONLY AFTER server RPC returns success; rolls back on error!
    @discardableResult
    public func setChatTTL(
        chatId: Int64,
        ttlSeconds: UInt32,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        let previousTTL = chats.first(where: { $0.chatId == chatId })?.ttlSeconds ?? 0
        do {
            let applied = try await client.setChatTTL(chatId: chatId, ttlSeconds: ttlSeconds)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].ttlSeconds = applied
            }
            persistSnapshotToDisk()
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.localized(.ttlUpdatedToast, locale: locale),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: false
            )
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].ttlSeconds = previousTTL
            }
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.localized(.ttlUpdateFailedToast, locale: locale),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: true
            )
            return false
        }
    }

    // MARK: - Folders & Forum Topics CRUD

    public func saveFolder(_ folder: FabushiFolder) async throws -> FabushiFolder {
        let capturedGen = coordinator.accountScopeGeneration
        let saved: FabushiFolder
        if folder.id > 0 {
            saved = try await client.updateFolder(folderId: folder.id, folder: folder)
        } else {
            saved = try await client.saveFolder(folder)
        }
        guard coordinator.accountScopeGeneration == capturedGen else { return saved }
        upsertFolder(saved)
        persistSnapshotToDisk()
        return saved
    }

    public func deleteFolder(folderId: Int64) async throws {
        let capturedGen = coordinator.accountScopeGeneration
        try await client.deleteFolder(folderId: folderId)
        guard coordinator.accountScopeGeneration == capturedGen else { return }
        folders.removeAll { $0.id == folderId }
        if selectedCustomFolderId == folderId {
            selectedCustomFolderId = nil
        }
        persistSnapshotToDisk()
    }

    public func loadTopics(chatId: Int64) async {
        let capturedGen = coordinator.accountScopeGeneration
        if let fetched = try? await client.listTopics(chatId: chatId),
           coordinator.accountScopeGeneration == capturedGen {
            topicsByChat[chatId] = fetched
        }
    }

    public func createTopic(chatId: Int64, title: String) async throws -> FabushiForumTopic {
        let capturedGen = coordinator.accountScopeGeneration
        let topic = try await client.createTopic(chatId: chatId, title: title)
        guard coordinator.accountScopeGeneration == capturedGen else { return topic }
        upsertTopic(topic, inChatId: chatId)
        return topic
    }

    public func updateTopic(
        chatId: Int64,
        topicId: Int64,
        title: String? = nil,
        isPinned: Bool? = nil,
        isClosed: Bool? = nil
    ) async throws {
        let capturedGen = coordinator.accountScopeGeneration
        _ = try await client.updateTopic(
            chatId: chatId,
            topicId: topicId,
            title: title,
            isPinned: isPinned,
            isClosed: isClosed
        )
        guard coordinator.accountScopeGeneration == capturedGen else { return }
        if var list = topicsByChat[chatId],
           let idx = list.firstIndex(where: { $0.topicId == topicId }) {
            if let title { list[idx].title = title }
            if let isPinned { list[idx].isPinned = isPinned }
            if let isClosed { list[idx].isClosed = isClosed }
            topicsByChat[chatId] = list
        }
    }

    public func deleteTopic(chatId: Int64, topicId: Int64) async throws {
        let capturedGen = coordinator.accountScopeGeneration
        _ = try await client.deleteTopic(chatId: chatId, topicId: topicId)
        guard coordinator.accountScopeGeneration == capturedGen else { return }
        topicsByChat[chatId]?.removeAll { $0.topicId == topicId }
    }

    // MARK: - Feature #9 + `D10-06`: Multi-Select Compose Engine

    public func beginComposeMode() {
        _ = reconcileAccountScopeIfNeeded()
        isComposeActive = true
        composeErrorMessage = nil
    }

    /// `D10-06`: Advances `composeEpoch`, clears `pendingCreationEpoch`, and resets compose state
    /// so any in-flight `chats.create` completion is strictly dropped.
    public func cancelComposeMode() {
        composeEpoch &+= 1
        pendingCreationEpoch = nil
        isComposeActive = false
        isCreatingComposeChat = false
        isResolvingComposePeer = false
        composeQuery = ""
        composeGroupTitle = ""
        selectedComposeRecipients.removeAll()
        resolvedComposePeer = nil
        composeErrorMessage = nil
    }

    public func toggleComposeRecipient(_ recipient: FabushiComposeRecipient) {
        if let idx = selectedComposeRecipients.firstIndex(where: { $0.userId == recipient.userId }) {
            selectedComposeRecipients.remove(at: idx)
        } else {
            selectedComposeRecipients.append(recipient)
        }
        composeErrorMessage = nil
    }

    public func removeComposeRecipient(userId: UInt64) {
        selectedComposeRecipients.removeAll { $0.userId == userId }
    }

    public func composeCandidates() -> (humans: [FabushiComposeRecipient], bots: [FabushiComposeRecipient]) {
        let query = composeQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let cleanedQuery = query.hasPrefix("@") ? String(query.dropFirst()) : query

        var humanList: [FabushiComposeRecipient] = contacts
            .filter { !$0.isBot }
            .map {
                FabushiComposeRecipient(
                    userId: $0.userId,
                    displayName: $0.displayName,
                    username: $0.username,
                    isBot: false,
                    subtitle: $0.username.map { "@\($0)" } ?? $0.phone
                )
            }

        var botById: [UInt64: FabushiComposeRecipient] = [:]
        for bot in bots {
            botById[bot.botId] = FabushiComposeRecipient(
                userId: bot.botId,
                displayName: bot.displayName.isEmpty ? (bot.username ?? "Bot \(bot.botId)") : bot.displayName,
                username: bot.username,
                isBot: true,
                subtitle: bot.descriptionText ?? bot.username.map { "@\($0)" }
            )
        }
        for contact in contacts where contact.isBot {
            if botById[contact.userId] == nil {
                botById[contact.userId] = FabushiComposeRecipient(
                    userId: contact.userId,
                    displayName: contact.displayName,
                    username: contact.username,
                    isBot: true,
                    subtitle: contact.username.map { "@\($0)" }
                )
            }
        }
        var botList = Array(botById.values).sorted { $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending }
        humanList.sort { $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending }

        if !cleanedQuery.isEmpty {
            humanList = humanList.filter {
                $0.displayName.lowercased().contains(cleanedQuery)
                    || ($0.username?.lowercased().contains(cleanedQuery) ?? false)
            }
            botList = botList.filter {
                $0.displayName.lowercased().contains(cleanedQuery)
                    || ($0.username?.lowercased().contains(cleanedQuery) ?? false)
            }
        }
        return (humanList, botList)
    }

    public func resolveComposeUsername() async {
        let raw = composeQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        let handle = raw.hasPrefix("@") ? String(raw.dropFirst()) : raw
        guard !handle.isEmpty else {
            resolvedComposePeer = nil
            return
        }
        let capturedEpoch = composeEpoch
        let capturedGen = coordinator.accountScopeGeneration
        isResolvingComposePeer = true
        defer {
            if composeEpoch == capturedEpoch && coordinator.accountScopeGeneration == capturedGen {
                isResolvingComposePeer = false
            }
        }
        do {
            let profile = try await client.resolvePeer(username: handle)
            guard composeEpoch == capturedEpoch,
                  isComposeActive,
                  coordinator.accountScopeGeneration == capturedGen
            else {
                return
            }
            resolvedComposePeer = profile
        } catch {
            if composeEpoch == capturedEpoch && coordinator.accountScopeGeneration == capturedGen {
                resolvedComposePeer = nil
            }
        }
    }

    /// Creates a DM or multi-member group atomically via `chats.create`, strictly guarded by `composeEpoch`
    /// (`D10-06`) and immediately subscribing via `chats.subscribeWs` (`D14-05`).
    @discardableResult
    public func createComposeConversation(
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> FabushiMessagingChat? {
        guard !selectedComposeRecipients.isEmpty else { return nil }
        if !isComposeActive {
            isComposeActive = true
        }
        let capturedEpoch = composeEpoch
        let capturedGen = coordinator.accountScopeGeneration
        pendingCreationEpoch = capturedEpoch
        isCreatingComposeChat = true
        composeErrorMessage = nil

        let recipients = selectedComposeRecipients
        let trimmedGroupTitle = composeGroupTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        let isGroupCreation = recipients.count > 1 || !trimmedGroupTitle.isEmpty
        let resolvedTitle: String
        if !trimmedGroupTitle.isEmpty {
            resolvedTitle = trimmedGroupTitle
        } else if recipients.count == 1 {
            resolvedTitle = recipients[0].displayName
        } else {
            resolvedTitle = recipients.prefix(3).map(\.displayName).joined(separator: ", ")
        }

        let request = FabushiCreateChatRequest(
            title: resolvedTitle,
            chatType: isGroupCreation ? 1 : (recipients.first?.isBot == true ? 3 : 0),
            peerUserId: isGroupCreation ? nil : recipients.first?.userId,
            memberIds: recipients.map(\.userId),
            memberUsernames: recipients.compactMap(\.username),
            clientNonce: "compose-\(nextClientNonce())"
        )

        do {
            var created = try await client.createChat(request)

            // `D10-06` strict late-response fence: drop if cancelled, epoch changed, or account switched!
            guard pendingCreationEpoch == capturedEpoch,
                  isComposeActive,
                  composeEpoch == capturedEpoch,
                  coordinator.accountScopeGeneration == capturedGen
            else {
                return nil
            }

            // `D14-04`: Multi-member groups with bots MUST keep `isGroup = true` and `isBot = false`.
            if isGroupCreation {
                created.isGroup = true
                created.isBot = false
                if created.kind.lowercased() == "bot" || created.kind.lowercased() == "private" {
                    created.kind = "group"
                }
            } else if recipients.first?.isBot == true {
                created.isBot = true
                created.isGroup = false
            }

            upsertChat(created)
            rebuildKnownBotIds()
            persistSnapshotToDisk()

            // `D14-05`: Immediately subscribe WS for newly created chat/group!
            _ = await subscribeWsIfNeeded(chatId: created.chatId, force: true)

            guard pendingCreationEpoch == capturedEpoch,
                  isComposeActive,
                  composeEpoch == capturedEpoch,
                  coordinator.accountScopeGeneration == capturedGen
            else {
                return nil
            }

            pendingCreationEpoch = nil
            isCreatingComposeChat = false
            isComposeActive = false
            composeEpoch &+= 1
            composeQuery = ""
            composeGroupTitle = ""
            selectedComposeRecipients.removeAll()
            resolvedComposePeer = nil
            activeChatId = created.chatId
            lastActiveChatId = created.chatId

            router?.navigate(to: .chats(chatId: created.chatId, messageId: nil), coordinator: coordinator)
            return created
        } catch {
            guard pendingCreationEpoch == capturedEpoch,
                  composeEpoch == capturedEpoch,
                  coordinator.accountScopeGeneration == capturedGen
            else {
                return nil
            }
            pendingCreationEpoch = nil
            isCreatingComposeChat = false
            composeErrorMessage = FabushiMessagingStrings.localized(.composeCreateFailed, locale: locale)
            return nil
        }
    }

    // MARK: - Feature #15 + `D10-07` + `D14-03`: Drafts & Durable Sending

    @discardableResult
    public func nextDraftOpGen(for chatId: Int64) -> UInt64 {
        let next = (draftOpGenByChat[chatId] ?? 0) &+ 1
        draftOpGenByChat[chatId] = next
        return next
    }

    public func updateComposerDraft(chatId: Int64, text: String, persistRemote: Bool = true) async {
        let capturedGen = coordinator.accountScopeGeneration
        let draftGen = nextDraftOpGen(for: chatId)
        unsentDraftTextByChat[chatId] = text

        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            draftsByChat.removeValue(forKey: chatId)
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].draft = nil
            }
            persistSnapshotToDisk()
            if persistRemote {
                _ = try? await client.clearDraft(chatId: chatId)
            }
            return
        }

        let draft = FabushiDraftRecord(
            chatId: chatId,
            text: text,
            replyToMsgId: replyTargetByChat[chatId]?.messageId,
            dateMs: FabushiWireDateParser.nowMs()
        )
        draftsByChat[chatId] = draft
        if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
            chats[idx].draft = draft
        }
        persistSnapshotToDisk()

        guard persistRemote else { return }
        if let saved = try? await client.saveDraft(draft),
           coordinator.accountScopeGeneration == capturedGen,
           draftOpGenByChat[chatId] == draftGen {
            draftsByChat[chatId] = saved
        }
    }

    /// Loads draft from host strictly fenced by `draftOpGenByChat` so stale fetches never clobber user input (`D10-07`).
    public func loadDraft(chatId: Int64) async {
        let capturedGen = coordinator.accountScopeGeneration
        let draftGen = nextDraftOpGen(for: chatId)
        guard let fetched = try? await client.getDraft(chatId: chatId),
              coordinator.accountScopeGeneration == capturedGen,
              draftOpGenByChat[chatId] == draftGen
        else {
            return
        }
        applyRemoteDraft(chatId: chatId, draft: fetched)
    }

    /// Primary composer send path (`D14-03` + `D10-07`):
    /// - Always dispatches via `messages.sendDurable` forwarding BOTH `reply_to` AND `quote_text`.
    /// - Restores unsent draft text + emits chat-scoped failure toast on permanent send failure iff
    ///   the user has not typed newer text (`draftOpGenByChat[chatId] == sendDraftGen`).
    @discardableResult
    public func sendComposerMessage(
        chatId: Int64,
        text: String,
        entities: [FabushiMessageEntity] = [],
        content: FabushiMessageContent? = nil,
        replyTarget: FabushiReplyTarget? = nil,
        topicId: Int64? = nil,
        silent: Bool = false,
        scheduledAtMs: Int64? = nil,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> FabushiSendMessageResponse? {
        let capturedGen = coordinator.accountScopeGeneration
        let activeReply = replyTarget ?? replyTargetByChat[chatId]
        let sendDraftGen = nextDraftOpGen(for: chatId)
        inFlightSendDraftGenByChat[chatId] = sendDraftGen

        // Clear composer draft optimistically
        unsentDraftTextByChat[chatId] = ""
        draftsByChat.removeValue(forKey: chatId)
        replyTargetByChat.removeValue(forKey: chatId)
        if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
            chats[idx].draft = nil
        }

        let nonce = nextClientNonce()
        let provisionalId = nextProvisionalMessageId()
        let mySenderId: UInt64
        if case .authenticated(let uid, _, _) = coordinator.authState {
            mySenderId = uid
        } else {
            mySenderId = 0
        }

        let provisionalMessage = FabushiMessageRecord(
            messageId: provisionalId,
            chatId: chatId,
            senderId: mySenderId,
            topicId: topicId,
            text: text,
            entities: entities,
            content: content ?? .text(text: text, entities: entities),
            dateMs: FabushiWireDateParser.nowMs(),
            isOutgoing: true,
            replyTo: activeReply.map {
                FabushiReplyHeader(
                    replyToMsgId: $0.messageId,
                    quoteText: $0.quoteText,
                    senderName: $0.senderName
                )
            },
            clientNonce: nonce,
            deliveryStatus: .sending,
            sendAsPeerId: selectedSendAsPeerByChat[chatId]
        )
        appendOrReplaceMessage(provisionalMessage)

        let request = FabushiRichSendMessageRequest(
            chatId: chatId,
            text: text,
            content: content,
            entities: entities,
            replyTo: activeReply?.messageId,
            quoteText: activeReply?.quoteText,
            topicId: topicId,
            clientNonce: nonce,
            silent: silent,
            scheduledAtMs: scheduledAtMs,
            sendAsPeerId: selectedSendAsPeerByChat[chatId]
        )

        do {
            let response = try await client.sendDurable(request)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            let delivery = FabushiMessageDeliveryStatus(wireStatus: response.status)
            reconcileProvisionalMessage(
                clientNonce: nonce,
                serverMessageId: response.messageId > 0 ? response.messageId : provisionalId,
                status: delivery
            )
            if response.pts > 0 {
                ptsByChat[chatId] = max(ptsByChat[chatId] ?? 0, response.pts)
            }
            updateChatPreview(
                chatId: chatId,
                messageId: response.messageId > 0 ? response.messageId : provisionalId,
                preview: text.isEmpty ? (content?.previewSummary ?? "") : text,
                dateMs: response.dateMs
            )
            _ = try? await client.clearDraft(chatId: chatId)
            persistSnapshotToDisk()
            return response
        } catch let error as FabushiCoordinatorError {
            if case .staleGeneration = error { return nil }
            return nil
        } catch let hostError as FabushiHostError {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if hostError.isCancelled || hostError.code == "stale_generation" { return nil }
            handleFailedComposerSend(
                chatId: chatId,
                clientNonce: nonce,
                originalText: text,
                originalReply: activeReply,
                sendDraftGen: sendDraftGen,
                router: router,
                isCompact: isCompact,
                locale: locale
            )
            return nil
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if error is CancellationError { return nil }
            handleFailedComposerSend(
                chatId: chatId,
                clientNonce: nonce,
                originalText: text,
                originalReply: activeReply,
                sendDraftGen: sendDraftGen,
                router: router,
                isCompact: isCompact,
                locale: locale
            )
            return nil
        }
    }

    /// Non-composer send path (`D10-07`): Bot Reply Keyboard taps, Stickers, GIFs, `/start`,
    /// Forwards, and Instant View Shares MUST NEVER clear or overwrite unsent composer drafts!
    @discardableResult
    public func sendNonComposerMessage(
        chatId: Int64,
        text: String? = nil,
        content: FabushiMessageContent? = nil,
        replyTo: Int64? = nil,
        quoteText: String? = nil,
        forwardFromChatId: Int64? = nil,
        forwardFromMsgId: Int64? = nil,
        topicId: Int64? = nil,
        silent: Bool = false,
        scheduledAtMs: Int64? = nil
    ) async throws -> FabushiSendMessageResponse {
        let capturedGen = coordinator.accountScopeGeneration
        let nonce = nextClientNonce()
        let request = FabushiRichSendMessageRequest(
            chatId: chatId,
            text: text,
            content: content,
            replyTo: replyTo,
            quoteText: quoteText,
            topicId: topicId,
            clientNonce: nonce,
            forwardFromChatId: forwardFromChatId,
            forwardFromMsgId: forwardFromMsgId,
            silent: silent,
            scheduledAtMs: scheduledAtMs,
            sendAsPeerId: selectedSendAsPeerByChat[chatId]
        )
        let response = try await client.sendDurable(request)
        if coordinator.accountScopeGeneration == capturedGen {
            let preview = text ?? content?.previewSummary ?? ""
            updateChatPreview(
                chatId: chatId,
                messageId: response.messageId,
                preview: preview,
                dateMs: response.dateMs
            )
            persistSnapshotToDisk()
        }
        return response
    }

    private func handleFailedComposerSend(
        chatId: Int64,
        clientNonce: UInt64,
        originalText: String,
        originalReply: FabushiReplyTarget?,
        sendDraftGen: UInt64,
        router: ShellNavigationRouter?,
        isCompact: Bool,
        locale: FabushiLocale
    ) {
        reconcileProvisionalMessage(clientNonce: clientNonce, serverMessageId: nil, status: .failed)
        // `D10-07`: Restore unsent text + reply ONLY if user has NOT edited the composer since send!
        if draftOpGenByChat[chatId] == sendDraftGen {
            unsentDraftTextByChat[chatId] = originalText
            if let originalReply {
                replyTargetByChat[chatId] = originalReply
            }
            let restored = FabushiDraftRecord(
                chatId: chatId,
                text: originalText,
                replyToMsgId: originalReply?.messageId,
                dateMs: FabushiWireDateParser.nowMs()
            )
            draftsByChat[chatId] = restored
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].draft = restored
            }
            persistSnapshotToDisk()
            Task { @MainActor [weak self] in
                _ = try? await self?.client.saveDraft(restored)
            }
        }
        _ = router?.showChatToastIfVisible(
            FabushiMessagingStrings.localized(.messageSendFailedToast, locale: locale),
            chatId: chatId,
            coordinator: coordinator,
            isCompact: isCompact,
            isError: true
        )
    }

    // MARK: - Feature #11 + `D10-08`: Contacts, Blocklist, Send-As & Prestart Bot

    public func refreshContactsAndBlocklist() async {
        _ = reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isLoadingContacts = true
        defer {
            if coordinator.accountScopeGeneration == capturedGen {
                isLoadingContacts = false
            }
        }

        async let contactsTask: [FabushiContact]? = try? await client.listContacts()
        async let blockedTask: [FabushiBlockedUser]? = try? await client.listBlockedUsers()
        async let botsTask: [FabushiBotConfig]? = try? await client.listBots()

        let (fetchedContacts, fetchedBlocked, fetchedBots) = await (contactsTask, blockedTask, botsTask)
        guard coordinator.accountScopeGeneration == capturedGen else { return }
        if let fetchedContacts {
            contacts = fetchedContacts
        }
        if let fetchedBlocked {
            blockedUsers = fetchedBlocked
        }
        if let fetchedBots {
            bots = fetchedBots
        }
        persistSnapshotToDisk()
    }

    @discardableResult
    public func addContact(
        firstName: String,
        lastName: String? = nil,
        username: String? = nil,
        phone: String? = nil,
        isBot: Bool = false,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> FabushiAddContactResponse? {
        let capturedGen = coordinator.accountScopeGeneration
        let cleanedUsername = username?.trimmingCharacters(in: .whitespacesAndNewlines)
        let normalizedUsername = (cleanedUsername?.hasPrefix("@") == true)
            ? String(cleanedUsername!.dropFirst())
            : cleanedUsername

        let request = FabushiAddContactRequest(
            firstName: firstName.trimmingCharacters(in: .whitespacesAndNewlines),
            lastName: lastName?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
            username: normalizedUsername?.nilIfEmpty,
            phone: phone?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
            isBot: isBot
        )
        do {
            let response = try await client.addContact(request)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            let newContact = FabushiContact(
                userId: response.userId,
                firstName: request.firstName,
                lastName: request.lastName,
                username: response.username.nilIfEmpty ?? request.username,
                phone: request.phone,
                isBot: response.isBot,
                isMutual: false,
                chatId: response.chatId
            )
            contacts.removeAll { $0.userId == newContact.userId }
            contacts.insert(newContact, at: 0)
            persistSnapshotToDisk()
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.contactAddedToast, locale: locale),
                isError: false
            )
            return response
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.contactAddFailedToast, locale: locale),
                isError: true
            )
            return nil
        }
    }

    @discardableResult
    public func deleteContact(
        userId: UInt64,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            try await client.deleteContact(userId: userId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            contacts.removeAll { $0.userId == userId }
            persistSnapshotToDisk()
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.contactDeletedToast, locale: locale),
                isError: false
            )
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.contactDeleteFailedToast, locale: locale),
                isError: true
            )
            return false
        }
    }

    @discardableResult
    public func resolveUsernamePeer(_ rawUsername: String) async -> FabushiUserProfile? {
        let trimmed = rawUsername.trimmingCharacters(in: .whitespacesAndNewlines)
        let handle = trimmed.hasPrefix("@") ? String(trimmed.dropFirst()) : trimmed
        guard !handle.isEmpty else {
            resolvedContactPeer = nil
            return nil
        }
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let profile = try await client.resolvePeer(username: handle)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            resolvedContactPeer = profile
            return profile
        } catch {
            if coordinator.accountScopeGeneration == capturedGen {
                resolvedContactPeer = nil
            }
            return nil
        }
    }

    /// `D10-08`: Authoritative block/unblock with post-RPC confirmation toast & rollback on failure.
    @discardableResult
    public func setUserBlocked(
        userId: UInt64,
        blocked: Bool,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        let previousBlocked = blockedUsers

        do {
            if blocked {
                let record = try await client.blockUser(userId: userId)
                guard coordinator.accountScopeGeneration == capturedGen else { return false }
                blockedUsers.removeAll { $0.userId == userId }
                blockedUsers.insert(record, at: 0)
                router?.toastController.showNonChatToast(
                    FabushiMessagingStrings.localized(.userBlockedToast, locale: locale),
                    isError: false
                )
            } else {
                try await client.unblockUser(userId: userId)
                guard coordinator.accountScopeGeneration == capturedGen else { return false }
                blockedUsers.removeAll { $0.userId == userId }
                router?.toastController.showNonChatToast(
                    FabushiMessagingStrings.localized(.userUnblockedToast, locale: locale),
                    isError: false
                )
            }
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            blockedUsers = previousBlocked
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.blockActionFailedToast, locale: locale),
                isError: true
            )
            return false
        }
    }

    public func isUserBlocked(_ userId: UInt64) -> Bool {
        blockedUsers.contains { $0.userId == userId }
    }

    public func selectSendAsPeer(chatId: Int64, peerId: UInt64?) {
        selectedSendAsPeerByChat[chatId] = peerId
    }

    @discardableResult
    public func loadBotPublicProfile(target: String) async -> FabushiBotPublicProfile? {
        let capturedGen = coordinator.accountScopeGeneration
        guard let profile = try? await client.getBotPublicProfile(target: target),
              coordinator.accountScopeGeneration == capturedGen
        else {
            return nil
        }
        botPublicProfilesByTarget[target] = profile
        return profile
    }

    /// Starts a Prestart Bot conversation (`miniapp.allowBotSendMessage` + non-composer `"/start"`
    /// preserving any unsent draft per `D10-07`).
    @discardableResult
    public func startPrestartBot(
        botId: UInt64,
        targetKey: String? = nil,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> Int64? {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let allowed = try await client.allowBotSendMessage(botId: botId)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            let targetChatId = allowed.chatId ?? Int64(bitPattern: botId)
            _ = try? await sendNonComposerMessage(chatId: targetChatId, text: "/start")
            _ = await subscribeWsIfNeeded(chatId: targetChatId, force: true)
            if let targetKey, let existing = botPublicProfilesByTarget[targetKey] {
                botPublicProfilesByTarget[targetKey] = FabushiBotPublicProfile(
                    bot: existing.bot,
                    mainAppId: existing.mainAppId,
                    existingChatId: targetChatId,
                    hasStartedChat: true,
                    allowsWriteToPm: true,
                    isBlocked: false,
                    isMuted: existing.isMuted
                )
            }
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.prestartBotStartedToast, locale: locale),
                isError: false
            )
            router?.navigate(to: .chats(chatId: targetChatId, messageId: nil), coordinator: coordinator)
            return targetChatId
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.prestartBotStartFailedToast, locale: locale),
                isError: true
            )
            return nil
        }
    }

    // MARK: - Feature #12: PTS/QTS Sync & Outbox Controls

    public var visibleOutboxEntries: [FabushiOutboxEntry] {
        outboxEntries.filter { !dismissedOutboxNonces.contains($0.clientNonce) && $0.status != .delivered }
    }

    public func nextClientNonce() -> UInt64 {
        nonceCounter &+= 1
        let nowPart = UInt64(max(1, FabushiWireDateParser.nowMs())) << 16
        let candidate = nowPart | (nonceCounter & 0xFFFF)
        return candidate == 0 ? 1 : candidate
    }

    public func refreshOutbox() async {
        let capturedGen = coordinator.accountScopeGeneration
        async let entriesTask: [FabushiOutboxEntry]? = try? await client.listOutbox()
        async let countTask: FabushiOutboxPendingCountResult? = try? await client.outboxPendingCount()
        let (entries, countRes) = await (entriesTask, countTask)
        guard coordinator.accountScopeGeneration == capturedGen else { return }
        if let entries {
            applyOutboxList(entries)
        }
        if let countRes {
            pendingOutboxCount = countRes.pendingCount
        }
    }

    @discardableResult
    public func retryAllOutbox() async -> Int {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            _ = try? await client.recoverInFlightOutbox()
            let resetCount = try await client.retryOutboxNow()
            guard coordinator.accountScopeGeneration == capturedGen else { return 0 }
            await refreshOutbox()
            return resetCount
        } catch {
            return 0
        }
    }

    public func dismissOutboxEntry(clientNonce: UInt64) {
        dismissedOutboxNonces.insert(clientNonce)
        outboxEntries.removeAll { $0.clientNonce == clientNonce }
        pendingOutboxCount = visibleOutboxEntries.count
    }

    public func syncDifferenceForChat(_ chatId: Int64) async {
        let capturedGen = coordinator.accountScopeGeneration
        isRecoveringSyncGaps = true
        defer {
            if coordinator.accountScopeGeneration == capturedGen {
                isRecoveringSyncGaps = false
            }
        }

        do {
            let diff = try await client.getSyncDifference(chatId: chatId, limit: 100)
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            ptsByChat[chatId] = max(ptsByChat[chatId] ?? 0, max(diff.pts, diff.currentPts))
            if !diff.newMessages.isEmpty {
                mergeFetchedHistory(diff.newMessages, forChatId: chatId)
                persistSnapshotToDisk()
            }
        } catch {
            // Sync difference errors are transient; keep last known PTS intact.
        }
    }

    public func syncAllPendingGaps() async {
        let capturedGen = coordinator.accountScopeGeneration
        isRecoveringSyncGaps = true
        defer {
            if coordinator.accountScopeGeneration == capturedGen {
                isRecoveringSyncGaps = false
            }
        }

        guard let diffs = try? await client.syncAllGaps(limit: 100),
              coordinator.accountScopeGeneration == capturedGen
        else {
            return
        }
        for diff in diffs {
            ptsByChat[diff.chatId] = max(ptsByChat[diff.chatId] ?? 0, max(diff.pts, diff.currentPts))
            if !diff.newMessages.isEmpty {
                mergeFetchedHistory(diff.newMessages, forChatId: diff.chatId)
            }
        }
        persistSnapshotToDisk()
    }

    // MARK: - Feature #18 + `BLK-08`: Unified Search (8 Backed Scopes, Zero Community)

    public func performSearch(
        query: String,
        scope: FabushiSearchScope? = nil,
        inChatId: Int64? = nil
    ) async {
        _ = reconcileAccountScopeIfNeeded()
        let activeScope = scope ?? searchScope
        searchScope = activeScope
        searchQuery = query

        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            searchResults = []
            isSearching = false
            isUsingOfflineSearchFallback = false
            return
        }

        let capturedGen = coordinator.accountScopeGeneration
        isSearching = true
        isUsingOfflineSearchFallback = false

        defer {
            if coordinator.accountScopeGeneration == capturedGen {
                isSearching = false
            }
        }

        var collected: [FabushiSearchResult] = []
        var seenIds = Set<String>()

        func appendUnique(_ result: FabushiSearchResult) {
            if seenIds.insert(result.id).inserted {
                collected.append(result)
            }
        }

        // Local matching chats for instant feedback under `.all` and `.chats`
        if inChatId == nil && (activeScope == .all || activeScope == .chats) {
            let lower = trimmed.lowercased()
            for chat in chats where chat.title.lowercased().contains(lower)
                || (chat.username?.lowercased().contains(lower) ?? false) {
                appendUnique(
                    FabushiSearchResult(
                        id: "chat:\(chat.chatId)",
                        kind: chat.isBot ? .bot : .chat,
                        chatId: chat.chatId,
                        title: chat.title,
                        snippet: chat.lastMessagePreview ?? (chat.username.map { "@\($0)" } ?? ""),
                        timestampMs: chat.lastMessageDateMs > 0 ? chat.lastMessageDateMs : nil,
                        isOfflineLocalHit: false
                    )
                )
            }
        }

        // Parallel real host search RPCs (strictly backed scopes — ZERO `community.*` RPCs per `BLK-08`)
        let wantsMessages = [.all, .messages, .media, .links, .files].contains(activeScope)
        let wantsCatalog = inChatId == nil && [.all, .chats, .bots, .miniApps].contains(activeScope)

        async let remoteMessagesTask: FabushiJSONValue? = wantsMessages
            ? (try? await client.searchRemoteMessages(
                query: trimmed,
                filter: activeScope.remoteMessageFilter,
                chatId: inChatId,
                limit: 50
            ))
            : nil
        async let catalogTask: FabushiJSONValue? = wantsCatalog
            ? (try? await client.searchGlobalCatalog(
                query: trimmed,
                category: activeScope.remoteCatalogCategory
            ))
            : nil
        async let localIndexTask: FabushiJSONValue? = wantsMessages
            ? (try? await client.searchLocalMessages(query: trimmed, chatId: inChatId, limit: 50))
            : nil
        async let sessionsTask: FabushiJSONValue? = (inChatId == nil && activeScope == .all)
            ? (try? await client.searchSessions(query: trimmed, limit: 10))
            : nil

        let (remoteMsgs, catalog, localIndex, sessions) = await (
            remoteMessagesTask,
            catalogTask,
            localIndexTask,
            sessionsTask
        )

        guard coordinator.accountScopeGeneration == capturedGen else { return }

        let targetMsgKind: FabushiSearchResultKind
        switch activeScope {
        case .media: targetMsgKind = .media
        case .links: targetMsgKind = .link
        case .files: targetMsgKind = .file
        default: targetMsgKind = .message
        }

        if let remoteMsgs, let hits = remoteMsgs["hits"]?.arrayValue {
            for hit in hits {
                let cid = hit["chat_id"]?.int64Value ?? 0
                let mid = hit["message_id"]?.int64Value ?? 0
                let text = hit["text"]?.stringValue ?? ""
                let dateMs = hit["date_ms"]?.int64Value
                let chatTitle = chats.first(where: { $0.chatId == cid })?.title ?? "Chat \(cid)"
                appendUnique(
                    FabushiSearchResult(
                        id: "msg:\(cid):\(mid)",
                        kind: targetMsgKind,
                        chatId: cid,
                        messageId: mid,
                        title: chatTitle,
                        snippet: text,
                        timestampMs: dateMs,
                        isOfflineLocalHit: false
                    )
                )
            }
        } else if wantsMessages {
            isUsingOfflineSearchFallback = true
        }

        if let catalog {
            if (activeScope == .all || activeScope == .bots), let botItems = catalog["bots"]?.arrayValue {
                for bot in botItems {
                    let bid = bot["bot_user_id"]?.uint64Value ?? bot["bot_id"]?.uint64Value ?? bot["user_id"]?.uint64Value ?? 0
                    let name = bot["display_name"]?.stringValue ?? bot["title"]?.stringValue ?? bot["username"]?.stringValue ?? "Bot"
                    let desc = bot["description"]?.stringValue ?? bot["username"]?.stringValue.map { "@\($0)" } ?? ""
                    appendUnique(
                        FabushiSearchResult(
                            id: "bot:\(bid):\(name)",
                            kind: .bot,
                            chatId: bid > 0 ? Int64(bitPattern: bid) : nil,
                            botUserId: bid > 0 ? bid : nil,
                            title: name,
                            snippet: desc,
                            isOfflineLocalHit: false
                        )
                    )
                }
            }
            if (activeScope == .all || activeScope == .miniApps), let appItems = catalog["apps"]?.arrayValue {
                for app in appItems {
                    let appId = app["app_id"]?.stringValue ?? app["short_name"]?.stringValue ?? app["id"]?.stringValue ?? ""
                    let title = app["title"]?.stringValue ?? app["name"]?.stringValue ?? appId
                    let desc = app["description"]?.stringValue ?? app["short_name"]?.stringValue ?? ""
                    guard !appId.isEmpty else { continue }
                    appendUnique(
                        FabushiSearchResult(
                            id: "miniapp:\(appId)",
                            kind: .miniApp,
                            miniAppId: appId,
                            title: title,
                            snippet: desc,
                            isOfflineLocalHit: false
                        )
                    )
                }
            }
            if let resultItems = catalog["results"]?.arrayValue {
                for item in resultItems {
                    let category = (item["category"]?.stringValue ?? item["kind"]?.stringValue ?? "").lowercased()
                    // `BLK-08`: Never emit unbacked community hits.
                    if category.contains("community") { continue }
                    let cid = item["chat_id"]?.int64Value ?? item["id"]?.int64Value
                    let mid = item["message_id"]?.int64Value
                    let title = item["title"]?.stringValue ?? item["peer_name"]?.stringValue ?? ""
                    let snippet = item["snippet"]?.stringValue ?? item["text"]?.stringValue ?? item["description"]?.stringValue ?? ""
                    let isBot = item["is_bot"]?.boolValue ?? (category == "bot" || category == "bots")
                    let appId = item["app_id"]?.stringValue ?? item["short_name"]?.stringValue

                    let kind: FabushiSearchResultKind
                    if appId != nil || category.contains("app") {
                        kind = .miniApp
                    } else if isBot {
                        kind = .bot
                    } else if (mid ?? 0) > 0 {
                        kind = .message
                    } else {
                        kind = .chat
                    }

                    if !matchesScope(kind: kind, scope: activeScope) { continue }
                    appendUnique(
                        FabushiSearchResult(
                            id: "cat:\(kind.rawValue):\(cid ?? 0):\(mid ?? 0):\(appId ?? title)",
                            kind: kind,
                            chatId: cid,
                            messageId: (mid ?? 0) > 0 ? mid : nil,
                            botUserId: item["bot_user_id"]?.uint64Value,
                            miniAppId: appId,
                            title: title.isEmpty ? snippet : title,
                            snippet: snippet,
                            timestampMs: FabushiWireDateParser.parseTimestampMs(from: item["date"] ?? item["date_ms"]),
                            isOfflineLocalHit: false
                        )
                    )
                }
            }
        }

        // Local Tantivy + SQLite FTS5/CJK offline hits
        if let localIndex, let hits = localIndex["hits"]?.arrayValue {
            for hit in hits {
                let cid = hit["chat_id"]?.int64Value ?? 0
                let mid = hit["message_id"]?.int64Value ?? 0
                let body = hit["body"]?.stringValue ?? ""
                let ts = hit["timestamp_ms"]?.int64Value
                let chatTitle = chats.first(where: { $0.chatId == cid })?.title ?? "Chat \(cid)"
                appendUnique(
                    FabushiSearchResult(
                        id: "msg:\(cid):\(mid)",
                        kind: targetMsgKind,
                        chatId: cid,
                        messageId: mid,
                        title: chatTitle,
                        snippet: body,
                        timestampMs: ts,
                        isOfflineLocalHit: true
                    )
                )
            }
        }

        // Hybrid agent sessions hits under `.all`
        if let sessions, let sessionHits = sessions.arrayValue {
            for item in sessionHits {
                let sid = item["session_id"]?.stringValue ?? item["id"]?.stringValue ?? UUID().uuidString
                let title = item["title"]?.stringValue ?? item["session_id"]?.stringValue ?? "Session"
                let snippet = item["snippet"]?.stringValue ?? item["content"]?.stringValue ?? ""
                appendUnique(
                    FabushiSearchResult(
                        id: "session:\(sid)",
                        kind: .sessionMemory,
                        title: title,
                        snippet: snippet,
                        isOfflineLocalHit: true
                    )
                )
            }
        }

        searchResults = collected
    }

    public func scheduleDebouncedSearch(query: String, scope: FabushiSearchScope? = nil, inChatId: Int64? = nil) {
        searchTask?.cancel()
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            searchQuery = query
            searchResults = []
            isSearching = false
            return
        }
        searchTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 180_000_000)
            guard !Task.isCancelled, let self else { return }
            await self.performSearch(query: query, scope: scope, inChatId: inChatId)
        }
    }

    // MARK: - Internal State Merging & Write-Through

    public func persistSnapshotToDisk(serverURL: String? = nil, accountId: String? = nil) {
        let resolvedAccount = accountId ?? coordinator.activeAccountId
        guard let resolvedAccount, !resolvedAccount.isEmpty else { return }
        let resolvedServer = serverURL ?? coordinator.settingsStore.activeServerURL
        let snapshot = FabushiSyncSnapshot(
            lastActiveChatId: lastActiveChatId ?? activeChatId,
            chats: chats,
            folders: folders,
            contacts: contacts,
            messagesByChat: messagesByChat,
            draftsByChat: draftsByChat,
            ptsByChat: ptsByChat
        )
        _ = cacheStore.saveSnapshot(snapshot, serverURL: resolvedServer, accountId: resolvedAccount)
    }

    public func upsertChat(_ chat: FabushiMessagingChat) {
        if let idx = chats.firstIndex(where: { $0.chatId == chat.chatId }) {
            chats[idx] = chats[idx].mergedPreservingRichLocalFlags(from: chat)
        } else {
            chats.append(chat)
        }
        chats = sortChatsDeterministically(chats)
        rebuildKnownBotIds()
    }

    private func rebuildKnownBotIds() {
        var ids = Set<Int64>()
        for chat in chats where chat.isBot && !chat.isGroup && !chat.isChannel {
            ids.insert(chat.chatId)
        }
        knownBotIds = ids
    }

    private func applyIncomingMessage(_ message: FabushiMessageRecord) {
        // `D14-04`: Incoming message clears any lingering `"thinking"` bot activity bubble in that chat!
        if let activity = activeBotActivityByChat[message.chatId], activity.isActive {
            activeBotActivityByChat.removeValue(forKey: message.chatId)
        }

        appendOrReplaceMessage(message)
        updateChatPreview(
            chatId: message.chatId,
            messageId: message.messageId,
            preview: message.text,
            dateMs: message.dateMs,
            incrementUnread: !message.isOutgoing && activeChatId != message.chatId
        )
        persistSnapshotToDisk()
    }

    private func applyEditedMessage(
        chatId: Int64,
        messageId: Int64,
        text: String,
        entities: [FabushiMessageEntity],
        editDateMs: Int64
    ) {
        updateMessage(chatId: chatId, messageId: messageId) { msg in
            msg.text = text
            if !entities.isEmpty {
                msg.entities = entities
            }
            msg.editDateMs = editDateMs
        }
        persistSnapshotToDisk()
    }

    private func applyDeletedMessages(chatId: Int64, messageIds: [Int64], forEveryone: Bool) {
        let idSet = Set(messageIds)
        guard var list = messagesByChat[chatId] else { return }
        if forEveryone {
            for idx in list.indices where idSet.contains(list[idx].messageId) {
                list[idx].isDeletedTombstone = true
                list[idx].text = ""
                list[idx].content = nil
            }
        } else {
            list.removeAll { idSet.contains($0.messageId) }
        }
        messagesByChat[chatId] = list
        persistSnapshotToDisk()
    }

    private func applyReactionsUpdate(chatId: Int64, messageId: Int64, reactions: [FabushiMessageReaction]) {
        updateMessage(chatId: chatId, messageId: messageId) { msg in
            msg.reactions = reactions
        }
        persistSnapshotToDisk()
    }

    private func applyRemoteDraft(chatId: Int64, draft: FabushiDraftRecord?) {
        if let draft, !draft.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            draftsByChat[chatId] = draft
            unsentDraftTextByChat[chatId] = draft.text
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].draft = draft
            }
        } else {
            draftsByChat.removeValue(forKey: chatId)
            unsentDraftTextByChat.removeValue(forKey: chatId)
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].draft = nil
            }
        }
    }

    private func applyOutboxList(_ list: [FabushiOutboxEntry]) {
        outboxEntries = list
            .filter { !dismissedOutboxNonces.contains($0.clientNonce) }
            .sorted { $0.createdAtMs > $1.createdAtMs }
        pendingOutboxCount = visibleOutboxEntries.count
    }

    private func applyOutboxStatusChanged(
        clientNonce: UInt64,
        chatId: Int64,
        status: FabushiOutboxStatus,
        serverMessageId: Int64?,
        retryCount: Int,
        lastError: String?
    ) {
        if let idx = outboxEntries.firstIndex(where: { $0.clientNonce == clientNonce }) {
            outboxEntries[idx].status = status
            outboxEntries[idx].retryCount = retryCount
            outboxEntries[idx].lastError = lastError
            outboxEntries[idx].serverMessageId = serverMessageId
            outboxEntries[idx].updatedAtMs = FabushiWireDateParser.nowMs()
        } else if status != .delivered && !dismissedOutboxNonces.contains(clientNonce) {
            outboxEntries.insert(
                FabushiOutboxEntry(
                    clientNonce: clientNonce,
                    chatId: chatId,
                    textPreview: messagesByChat[chatId]?.first(where: { $0.clientNonce == clientNonce })?.text ?? "",
                    status: status,
                    retryCount: retryCount,
                    lastError: lastError,
                    serverMessageId: serverMessageId
                ),
                at: 0
            )
        }
        pendingOutboxCount = visibleOutboxEntries.count

        let mappedDelivery: FabushiMessageDeliveryStatus
        switch status {
        case .pending: mappedDelivery = .pending
        case .inFlight: mappedDelivery = .sending
        case .delivered: mappedDelivery = .sent
        case .failed: mappedDelivery = .failed
        }
        reconcileProvisionalMessage(
            clientNonce: clientNonce,
            serverMessageId: serverMessageId,
            status: mappedDelivery
        )
    }

    private func mergeFetchedHistory(_ fetched: [FabushiMessageRecord], forChatId chatId: Int64) {
        var existing = messagesByChat[chatId] ?? []
        var byId: [Int64: FabushiMessageRecord] = [:]
        var byNonce: [UInt64: Int64] = [:]
        for item in existing {
            byId[item.messageId] = item
            if let nonce = item.clientNonce {
                byNonce[nonce] = item.messageId
            }
        }

        for var incoming in fetched {
            if incoming.chatId == 0 { incoming.chatId = chatId }
            if case .authenticated(let myUid, _, _) = coordinator.authState,
               incoming.senderId == myUid {
                incoming.isOutgoing = true
            }
            if let nonce = incoming.clientNonce, let oldId = byNonce[nonce], oldId != incoming.messageId {
                byId.removeValue(forKey: oldId)
            }
            byId[incoming.messageId] = incoming
        }

        existing = Array(byId.values).sorted { lhs, rhs in
            if lhs.dateMs != rhs.dateMs {
                return lhs.dateMs < rhs.dateMs
            }
            return lhs.messageId < rhs.messageId
        }
        messagesByChat[chatId] = existing
        loadedHistoryChats.insert(chatId)
    }

    private func appendOrReplaceMessage(_ message: FabushiMessageRecord) {
        var list = messagesByChat[message.chatId] ?? []
        if let nonce = message.clientNonce,
           let idx = list.firstIndex(where: { $0.clientNonce == nonce }) {
            list[idx] = message
        } else if let idx = list.firstIndex(where: { $0.messageId == message.messageId }) {
            list[idx] = message
        } else {
            list.append(message)
        }
        list.sort { lhs, rhs in
            if lhs.dateMs != rhs.dateMs {
                return lhs.dateMs < rhs.dateMs
            }
            return lhs.messageId < rhs.messageId
        }
        messagesByChat[message.chatId] = list
    }

    private func reconcileProvisionalMessage(
        clientNonce: UInt64,
        serverMessageId: Int64?,
        status: FabushiMessageDeliveryStatus
    ) {
        for (chatId, var list) in messagesByChat {
            if let idx = list.firstIndex(where: { $0.clientNonce == clientNonce }) {
                if let serverMessageId, serverMessageId > 0 {
                    list[idx].messageId = serverMessageId
                }
                list[idx].deliveryStatus = status
                messagesByChat[chatId] = list
                return
            }
        }
    }

    private func updateMessage(
        chatId: Int64,
        messageId: Int64,
        mutate: (inout FabushiMessageRecord) -> Void
    ) {
        guard var list = messagesByChat[chatId],
              let idx = list.firstIndex(where: { $0.messageId == messageId })
        else {
            return
        }
        mutate(&list[idx])
        messagesByChat[chatId] = list
    }

    private func updateChatPreview(
        chatId: Int64,
        messageId: Int64,
        preview: String,
        dateMs: Int64,
        incrementUnread: Bool = false
    ) {
        if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
            if messageId > 0 {
                chats[idx].lastMessageId = max(chats[idx].lastMessageId, messageId)
            }
            if !preview.isEmpty {
                chats[idx].lastMessagePreview = preview
            }
            if dateMs > 0 {
                chats[idx].lastMessageDateMs = max(chats[idx].lastMessageDateMs, dateMs)
            }
            if incrementUnread {
                chats[idx].unreadCount += 1
            }
            chats = sortChatsDeterministically(chats)
        }
    }

    private func upsertFolder(_ folder: FabushiFolder) {
        folders.removeAll { $0.id == folder.id }
        folders.append(folder)
        folders.sort { $0.sortOrder < $1.sortOrder }
    }

    private func upsertTopic(_ topic: FabushiForumTopic, inChatId chatId: Int64) {
        var list = topicsByChat[chatId] ?? []
        list.removeAll { $0.topicId == topic.topicId }
        list.append(topic)
        list.sort { lhs, rhs in
            if lhs.isPinned != rhs.isPinned { return lhs.isPinned && !rhs.isPinned }
            return lhs.createdAtMs > rhs.createdAtMs
        }
        topicsByChat[chatId] = list
    }

    private func upsertStory(_ story: FabushiStoryItem) {
        stories.removeAll { $0.storyId == story.storyId }
        stories.insert(story, at: 0)
    }

    private func matchesScope(kind: FabushiSearchResultKind, scope: FabushiSearchScope) -> Bool {
        switch scope {
        case .all: return true
        case .chats: return kind == .chat
        case .messages: return kind == .message
        case .media: return kind == .media
        case .links: return kind == .link
        case .files: return kind == .file
        case .bots: return kind == .bot
        case .miniApps: return kind == .miniApp
        }
    }

    private func isCurrentUser(_ senderId: Int64) -> Bool {
        guard senderId > 0, case .authenticated(let uid, _, _) = coordinator.authState else {
            return false
        }
        return UInt64(senderId) == uid
    }

    private func nextProvisionalMessageId() -> Int64 {
        let next = provisionalIdCounter
        provisionalIdCounter -= 1
        return next
    }

    // MARK: - M3b Stage Actions, Reactions, Polls, Todos, Pins, Export & Approvals (`#13`, `#14`, `D10-08`)

    @discardableResult
    public func editMessage(
        chatId: Int64,
        messageId: Int64,
        newText: String,
        entities: [FabushiMessageEntity] = [],
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let trimmed = newText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        let capturedGen = coordinator.accountScopeGeneration
        let previous = messagesByChat[chatId]?.first(where: { $0.messageId == messageId })
        do {
            try await client.editMessage(chatId: chatId, messageId: messageId, text: newText, entities: entities)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            applyEditedMessage(
                chatId: chatId,
                messageId: messageId,
                text: newText,
                entities: entities,
                editDateMs: FabushiWireDateParser.nowMs()
            )
            editingMessageByChat.removeValue(forKey: chatId)
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            if let previous {
                updateMessage(chatId: chatId, messageId: messageId) { msg in
                    msg.text = previous.text
                    msg.entities = previous.entities
                }
            }
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.localized(.messageSendFailedToast, locale: locale),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: true
            )
            return false
        }
    }

    @discardableResult
    public func deleteMessages(
        chatId: Int64,
        messageIds: [Int64],
        forEveryone: Bool = true,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Bool {
        guard !messageIds.isEmpty else { return false }
        let capturedGen = coordinator.accountScopeGeneration
        var succeededIds: [Int64] = []
        for mid in messageIds {
            if mid < 0 {
                succeededIds.append(mid)
                continue
            }
            do {
                try await client.deleteMessage(chatId: chatId, messageId: mid, revoke: forEveryone)
                succeededIds.append(mid)
            } catch {
                // Keep authoritative state for items that failed remote deletion.
            }
        }
        guard coordinator.accountScopeGeneration == capturedGen else { return false }
        if !succeededIds.isEmpty {
            applyDeletedMessages(chatId: chatId, messageIds: succeededIds, forEveryone: forEveryone)
            return succeededIds.count == messageIds.count
        }
        _ = router?.showChatToastIfVisible(
            FabushiMessagingStrings.localized(.messageSendFailedToast, locale: locale),
            chatId: chatId,
            coordinator: coordinator,
            isCompact: isCompact,
            isError: true
        )
        return false
    }

    /// Multi-forward via `messages.sendDurable` (`sendNonComposerMessage` preserves target chat's unsent draft per `D10-07`).
    @discardableResult
    public func forwardMessages(
        fromChatId: Int64,
        messageIds: [Int64],
        toChatId: Int64,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Int {
        guard !messageIds.isEmpty else { return 0 }
        let capturedGen = coordinator.accountScopeGeneration
        let sourceList = messagesByChat[fromChatId] ?? []
        let sourceById = Dictionary(uniqueKeysWithValues: sourceList.map { ($0.messageId, $0) })
        var sentCount = 0
        for mid in messageIds {
            let sourceText = sourceById[mid]?.text
            do {
                _ = try await sendNonComposerMessage(
                    chatId: toChatId,
                    text: sourceText,
                    forwardFromChatId: fromChatId,
                    forwardFromMsgId: mid
                )
                guard coordinator.accountScopeGeneration == capturedGen else { return sentCount }
                sentCount += 1
            } catch {
                // Continue forwarding remaining selected messages.
            }
        }
        if sentCount > 0 {
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.format(.forwardSentToastFormat, locale: locale, sentCount),
                chatId: fromChatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: false
            )
        }
        return sentCount
    }

    /// PTS-fenced reaction toggle (`messages.react`, preserving `is_chosen_by_me` and `chosen`).
    @discardableResult
    public func toggleReaction(chatId: Int64, messageId: Int64, emoji: String) async -> [FabushiMessageReaction] {
        let capturedGen = coordinator.accountScopeGeneration
        let previousReactions = messagesByChat[chatId]?.first(where: { $0.messageId == messageId })?.reactions ?? []
        let alreadyChosen = previousReactions.first(where: { $0.emoji == emoji })?.isChosenByMe ?? false

        do {
            let updated = try await client.reactToMessage(
                chatId: chatId,
                messageId: messageId,
                emoji: emoji,
                action: alreadyChosen ? "remove" : "add"
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return previousReactions }
            applyReactionsUpdate(chatId: chatId, messageId: messageId, reactions: updated)
            return updated
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return previousReactions }
            applyReactionsUpdate(chatId: chatId, messageId: messageId, reactions: previousReactions)
            return previousReactions
        }
    }

    @discardableResult
    public func pinMessage(
        chatId: Int64,
        messageId: Int64,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let record = try await client.pinMessage(chatId: chatId, messageId: messageId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            var list = pinnedMessagesByChat[chatId] ?? []
            list.removeAll { $0.messageId == messageId }
            list.insert(record, at: 0)
            pinnedMessagesByChat[chatId] = list
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].pinnedMessageId = messageId
            }
            persistSnapshotToDisk()
            router?.toastController.showStagePill(
                FabushiMessagingStrings.localized(.messagePinnedPill, locale: locale),
                chatId: chatId
            )
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    public func unpinMessage(
        chatId: Int64,
        messageId: Int64,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            try await client.unpinMessage(chatId: chatId, messageId: messageId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            pinnedMessagesByChat[chatId]?.removeAll { $0.messageId == messageId }
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].pinnedMessageId = pinnedMessagesByChat[chatId]?.first?.messageId
            }
            persistSnapshotToDisk()
            router?.toastController.showStagePill(
                FabushiMessagingStrings.localized(.messageUnpinnedPill, locale: locale),
                chatId: chatId
            )
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    public func createPoll(
        chatId: Int64,
        question: String,
        options: [String],
        isAnonymous: Bool = true,
        multipleChoice: Bool = false,
        isQuiz: Bool = false,
        correctOptionId: UInt8? = nil,
        solution: String? = nil,
        topicId: Int64? = nil
    ) async -> FabushiPollCard? {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let poll = try await client.createPoll(
                chatId: chatId,
                question: question,
                options: options,
                isAnonymous: isAnonymous,
                multipleChoice: multipleChoice,
                isQuiz: isQuiz,
                correctOptionId: correctOptionId,
                solution: solution,
                topicId: topicId,
                clientNonce: nextClientNonce()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            let mySenderId: UInt64
            if case .authenticated(let uid, _, _) = coordinator.authState {
                mySenderId = uid
            } else {
                mySenderId = 0
            }
            let message = FabushiMessageRecord(
                messageId: Int64(bitPattern: poll.pollId > 0 ? poll.pollId : UInt64(bitPattern: nextProvisionalMessageId())),
                chatId: chatId,
                senderId: mySenderId,
                topicId: topicId,
                text: poll.question,
                content: .poll(poll),
                dateMs: FabushiWireDateParser.nowMs(),
                isOutgoing: true,
                deliveryStatus: .sent
            )
            appendOrReplaceMessage(message)
            persistSnapshotToDisk()
            return poll
        } catch {
            return nil
        }
    }

    @discardableResult
    public func votePoll(chatId: Int64, messageId: Int64, pollId: UInt64, optionIds: [UInt8]) async -> FabushiPollCard? {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let updated = try await client.votePoll(pollId: pollId, optionIds: optionIds)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            updateMessage(chatId: chatId, messageId: messageId) { msg in
                msg.content = .poll(updated)
            }
            persistSnapshotToDisk()
            return updated
        } catch {
            return nil
        }
    }

    @discardableResult
    public func closePoll(chatId: Int64, messageId: Int64, pollId: UInt64) async -> FabushiPollCard? {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let closed = try await client.closePoll(pollId: pollId)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            updateMessage(chatId: chatId, messageId: messageId) { msg in
                msg.content = .poll(closed)
            }
            persistSnapshotToDisk()
            return closed
        } catch {
            return nil
        }
    }

    public func toggleTodoItem(chatId: Int64, messageId: Int64, itemId: String) {
        updateMessage(chatId: chatId, messageId: messageId) { msg in
            guard case .todo(var card) = msg.content else { return }
            for idx in card.items.indices where card.items[idx].id == itemId {
                card.items[idx].completed.toggle()
            }
            let updatedCard = FabushiTodoCard(
                id: card.id,
                title: card.title,
                items: card.items,
                completedCount: card.items.filter(\.completed).count,
                actionButtons: card.actionButtons
            )
            msg.content = .todo(updatedCard)
        }
        persistSnapshotToDisk()
    }

    @discardableResult
    public func clearChatHistory(
        chatId: Int64,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            try await client.clearChatHistory(chatId: chatId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            messagesByChat[chatId] = []
            if let idx = chats.firstIndex(where: { $0.chatId == chatId }) {
                chats[idx].lastMessagePreview = nil
                chats[idx].unreadCount = 0
            }
            persistSnapshotToDisk()
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.localized(.clearHistoryDoneToast, locale: locale),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: false
            )
            return true
        } catch {
            return false
        }
    }

    @discardableResult
    public func leaveChat(
        chatId: Int64,
        router: ShellNavigationRouter? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            try await client.leaveChat(chatId: chatId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            chats.removeAll { $0.chatId == chatId }
            messagesByChat.removeValue(forKey: chatId)
            pinnedMessagesByChat.removeValue(forKey: chatId)
            subscribedWsChatIds.remove(chatId)
            if activeChatId == chatId {
                activeChatId = nil
            }
            persistSnapshotToDisk()
            router?.dismissInspector()
            router?.navigateBackToChatList(coordinator: coordinator)
            return true
        } catch {
            router?.toastController.showNonChatToast(
                FabushiMessagingStrings.localized(.inspectorSaveFailedToast, locale: locale),
                isError: true
            )
            return false
        }
    }

    public func exportChatHistoryJSON(chatId: Int64) async throws -> String {
        do {
            return try await client.exportChatHistory(chatId: chatId, format: "json")
        } catch let hostError as FabushiHostError {
            if hostError.code == "stale_generation" || hostError.isCancelled {
                throw hostError
            }
            let localMessages = messagesByChat[chatId] ?? []
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            let data = try encoder.encode(localMessages)
            return String(data: data, encoding: .utf8) ?? "[]"
        }
    }

    /// `D10-08`: Authoritative media download — NEVER reports `"Saved"` on failure; emits localized
    /// `"Download failed: <name>"` chat-scoped error toast when download fails!
    public func downloadAttachment(
        fileId: String,
        fileName: String,
        chatId: Int64,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Data? {
        let capturedGen = coordinator.accountScopeGeneration
        if let cached = try? await client.casGet(fileId: fileId), !cached.isEmpty {
            return cached
        }
        do {
            let bytes = try await client.downloadMedia(fileId: fileId)
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            try? await client.casPut(fileId: fileId, data: bytes)
            return bytes
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            let displayName = fileName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? fileId : fileName
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.format(.downloadFailedToastFormat, locale: locale, displayName),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: true
            )
            return nil
        }
    }

    /// `D10-07`: Re-checks `coordinator.accountScopeGeneration == capturedGen` between `media.upload`
    /// and `messages.sendDurable` so an account switch mid-upload never leaks media into another account!
    @discardableResult
    public func uploadAndSendAttachment(
        chatId: Int64,
        fileName: String,
        mimeType: String,
        data: Data,
        caption: String? = nil,
        topicId: Int64? = nil,
        silent: Bool = false,
        scheduledAtMs: Int64? = nil,
        contentBuilder: (FabushiMediaUploadResponse) -> FabushiMessageContent,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> FabushiSendMessageResponse? {
        let capturedGen = coordinator.accountScopeGeneration
        let capturedAccount = coordinator.activeAccountId
        do {
            let uploaded = try await client.uploadMedia(
                filename: fileName,
                mimeType: mimeType,
                dataB64: data.base64EncodedString()
            )
            // Strict `D10-07` generation & account fence between upload and send!
            guard coordinator.accountScopeGeneration == capturedGen,
                  coordinator.activeAccountId == capturedAccount
            else {
                return nil
            }
            try? await client.casPut(fileId: uploaded.fileId, data: data, mimeType: mimeType)
            let content = contentBuilder(uploaded)
            let response = try await sendNonComposerMessage(
                chatId: chatId,
                text: caption,
                content: content,
                topicId: topicId,
                silent: silent,
                scheduledAtMs: scheduledAtMs
            )
            if coordinator.accountScopeGeneration == capturedGen {
                let mySenderId: UInt64
                if case .authenticated(let uid, _, _) = coordinator.authState {
                    mySenderId = uid
                } else {
                    mySenderId = 0
                }
                let record = FabushiMessageRecord(
                    messageId: response.messageId > 0 ? response.messageId : nextProvisionalMessageId(),
                    chatId: chatId,
                    senderId: mySenderId,
                    topicId: topicId,
                    text: caption ?? content.previewSummary,
                    content: content,
                    dateMs: response.dateMs,
                    isOutgoing: true,
                    clientNonce: response.clientNonce,
                    deliveryStatus: FabushiMessageDeliveryStatus(wireStatus: response.status),
                    sendAsPeerId: selectedSendAsPeerByChat[chatId]
                )
                appendOrReplaceMessage(record)
                persistSnapshotToDisk()
            }
            return response
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.localized(.messageSendFailedToast, locale: locale),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: true
            )
            return nil
        }
    }

    // MARK: - Approvals, Floor Control, Slash Commands (`D11-01`) & Inline Keyboard (`D11-02`, `D10-08`)

    public func refreshApprovals(sessionId: String? = nil) async {
        let capturedGen = coordinator.accountScopeGeneration
        if let items = try? await client.listApprovals(sessionId: sessionId, pendingOnly: true),
           coordinator.accountScopeGeneration == capturedGen {
            pendingApprovals = items
        }
    }

    @discardableResult
    public func decideApproval(
        approvalId: String,
        approved: Bool,
        router: ShellNavigationRouter? = nil,
        chatId: Int64? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        let status = approved ? "approved_once" : "denied"
        do {
            try await client.decideApproval(approvalId: approvalId, status: status)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            pendingApprovals.removeAll { $0.id == approvalId }
            return true
        } catch {
            if let chatId {
                _ = router?.showChatToastIfVisible(
                    FabushiMessagingStrings.localized(.inspectorSaveFailedToast, locale: locale),
                    chatId: chatId,
                    coordinator: coordinator,
                    isCompact: isCompact,
                    isError: true
                )
            }
            return false
        }
    }

    public func preemptFloor(chatId: Int64) async {
        let capturedGen = coordinator.accountScopeGeneration
        try? await client.preemptFloorForUser(chatId: chatId)
        if coordinator.accountScopeGeneration == capturedGen {
            floorSpeakerByChat.removeValue(forKey: chatId)
        }
    }

    public func refreshSlashCommands() async {
        let capturedGen = coordinator.accountScopeGeneration
        if let commands = try? await client.listSlashCommands(),
           coordinator.accountScopeGeneration == capturedGen {
            slashCommands = commands
        }
    }

    /// `D11-01`: Dispatches slash commands (including `/distill` & `/distill-script`) through real
    /// `fabushi.runCommand` and processes every `CommandUiEffect` cleanly.
    @discardableResult
    public func executeSlashCommand(
        _ commandText: String,
        chatId: Int64,
        botId: UInt64? = nil,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> FabushiCommandExecutionResponse? {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let response = try await client.runSlashCommand(
                command: commandText,
                sessionId: nil,
                chatId: chatId,
                botId: botId
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            applyCommandEffects(
                response.effects,
                chatId: chatId,
                router: router,
                isCompact: isCompact,
                locale: locale
            )
            let displayText = response.replyText ?? response.output
            if !displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let botMsg = FabushiMessageRecord(
                    messageId: nextProvisionalMessageId(),
                    chatId: chatId,
                    senderId: botId ?? 0,
                    text: displayText,
                    content: .text(text: displayText, entities: []),
                    dateMs: FabushiWireDateParser.nowMs(),
                    isOutgoing: false,
                    inlineKeyboard: response.inlineKeyboard,
                    deliveryStatus: .sent
                )
                appendOrReplaceMessage(botMsg)
                persistSnapshotToDisk()
            }
            return response
        } catch {
            return nil
        }
    }

    public func applyCommandEffects(
        _ rawEffects: [FabushiJSONValue],
        chatId: Int64,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) {
        for raw in rawEffects {
            guard let effect = FabushiCommandUiEffect.parse(from: raw) else { continue }
            switch effect {
            case .distillBrowserScript(let script):
                var existing = distilledScriptsByChat[chatId] ?? []
                existing.removeAll { $0.id == script.id }
                existing.insert(script, at: 0)
                distilledScriptsByChat[chatId] = existing
                router?.toastController.showStagePill(
                    "\(FabushiMessagingStrings.localized(.slashDistillCardTitle, locale: locale)): \(script.name)",
                    chatId: chatId
                )
            case .openRemoteComputer:
                router?.navigate(to: .remoteComputer(nodeId: nil), coordinator: coordinator)
            case .openHermesStudio(let tab):
                router?.navigate(to: .agentStudio(tab: tab), coordinator: coordinator)
            case .openTalkMode:
                router?.navigate(to: .calls, coordinator: coordinator)
            case .clearChatDisplay:
                messagesByChat[chatId] = []
            case .refreshSessionState:
                Task { @MainActor [weak self] in
                    await self?.refreshWorkspace(trigger: .manual)
                }
            case .showDiff:
                router?.navigate(to: .agentStudio(tab: "coding"), coordinator: coordinator)
            case .exportFile, .unknown:
                break
            }
        }
    }

    /// `D11-02` & `D10-08` & TRAP #5 honest inline keyboard router (zero fake successes).
    public func handleInlineKeyboardAction(
        button: FabushiInlineKeyboardButton,
        chatId: Int64,
        messageId: Int64,
        router: ShellNavigationRouter,
        isCompact: Bool,
        locale: FabushiLocale
    ) async {
        if let webAppURL = button.webAppUrl, !webAppURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            router.navigate(to: .miniApp(id: webAppURL), coordinator: coordinator)
            return
        }

        if let urlString = button.url, !urlString.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            let doc = FabushiInstantViewDocument(
                url: urlString,
                siteName: URL(string: urlString)?.host ?? urlString,
                title: button.text,
                blocks: [.paragraph(urlString)]
            )
            overlayController.openInstantView(doc)
            return
        }

        guard let callback = button.callbackData?.trimmingCharacters(in: .whitespacesAndNewlines),
              !callback.isEmpty
        else {
            return
        }

        // `D10-08`: `computer:` -> Remote Computers Hub navigation
        if callback.hasPrefix("computer:") {
            let nodeId = String(callback.dropFirst("computer:".count)).trimmingCharacters(in: .whitespacesAndNewlines)
            router.navigate(
                to: .remoteComputer(nodeId: nodeId.isEmpty ? nil : nodeId),
                coordinator: coordinator
            )
            return
        }

        // `D11-02`: `browser_takeover:<sid>`, `browser_resume:<sid>`, `browser_cancel:<sid>`
        if callback.hasPrefix("browser_takeover:")
            || callback.hasPrefix("browser_resume:")
            || callback.hasPrefix("browser_cancel:")
        {
            lastBrowserTakeoverAction = callback
            let response = try? await client.runSlashCommand(
                command: "/\(callback)",
                sessionId: nil,
                chatId: chatId,
                botId: nil
            )
            if let response, response.handled {
                applyCommandEffects(
                    response.effects,
                    chatId: chatId,
                    router: router,
                    isCompact: isCompact,
                    locale: locale
                )
            } else {
                _ = router.showChatToastIfVisible(
                    FabushiMessagingStrings.localized(.browserTakeoverUnavailableToast, locale: locale),
                    chatId: chatId,
                    coordinator: coordinator,
                    isCompact: isCompact,
                    isError: false
                )
            }
            return
        }

        if callback.hasPrefix("/") {
            _ = await executeSlashCommand(
                callback,
                chatId: chatId,
                botId: nil,
                router: router,
                isCompact: isCompact,
                locale: locale
            )
            return
        }

        // TRAP #5: Honest non-destructive notice for unmapped server callback queries (M6 `messages.callback`).
        _ = router.showChatToastIfVisible(
            FabushiMessagingStrings.localized(.inlineCallbackPendingNotice, locale: locale),
            chatId: chatId,
            coordinator: coordinator,
            isCompact: isCompact,
            isError: false
        )
    }

    // MARK: - Stories (`#20`)

    public func refreshStories() async {
        let capturedGen = coordinator.accountScopeGeneration
        if let fetched = try? await client.listStories(),
           coordinator.accountScopeGeneration == capturedGen {
            stories = fetched
        }
    }

    @discardableResult
    public func createStory(
        mediaId: String? = nil,
        mediaType: String = "photo",
        caption: String?,
        privacy: String = "everyone"
    ) async -> FabushiStoryItem? {
        let capturedGen = coordinator.accountScopeGeneration
        guard let created = try? await client.createStory(
            mediaId: mediaId,
            mediaType: mediaType,
            caption: caption,
            privacy: privacy
        ),
        coordinator.accountScopeGeneration == capturedGen
        else {
            return nil
        }
        upsertStory(created)
        return created
    }

    @discardableResult
    public func deleteStory(storyId: String) async -> Bool {
        let capturedGen = coordinator.accountScopeGeneration
        do {
            try await client.deleteStory(storyId: storyId)
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            stories.removeAll { $0.storyId == storyId }
            return true
        } catch {
            return false
        }
    }

    // MARK: - E2EE Safety Numbers & Fail-Closed Send (`#19`)

    public func loadE2EESafetyInfo(
        chatId: Int64,
        peerTitle: String,
        remoteIdentityKeyB64: String? = nil
    ) async throws -> FabushiE2EESafetyInfo {
        let capturedGen = coordinator.accountScopeGeneration
        let identity = try await client.e2eeIdentity()
        let peerKey = (remoteIdentityKeyB64?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false)
            ? remoteIdentityKeyB64!
            : identity.x25519PubB64
        let emojis = try await client.e2eeSafetyFingerprint(
            localIdentityKeyB64: identity.x25519PubB64,
            remoteIdentityKeyB64: peerKey
        )
        let qrPayload = "fabushi-e2ee://v1/\(identity.deviceId)/\(identity.x25519PubB64)/\(peerKey)"
        let info = FabushiE2EESafetyInfo(
            chatId: chatId,
            peerTitle: peerTitle,
            localIdentityKeyB64: identity.x25519PubB64,
            remoteIdentityKeyB64: remoteIdentityKeyB64,
            safetyEmojis: emojis,
            qrPayload: qrPayload,
            isVerified: verifiedE2EEChatIds.contains(chatId),
            peerEnrolledOnServer: remoteIdentityKeyB64 != nil
        )
        if coordinator.accountScopeGeneration == capturedGen {
            e2eeSafetyByChat[chatId] = info
        }
        return info
    }

    public func markE2EEVerified(chatId: Int64, verified: Bool) {
        if verified {
            verifiedE2EEChatIds.insert(chatId)
        } else {
            verifiedE2EEChatIds.remove(chatId)
        }
        if var info = e2eeSafetyByChat[chatId] {
            info.isVerified = verified
            e2eeSafetyByChat[chatId] = info
        }
        if var active = overlayController.presentedSafetyInfo, active.chatId == chatId {
            active.isVerified = verified
            overlayController.presentedSafetyInfo = active
        }
    }

    /// Strictly fail-closed E2EE send: NEVER silently downgrades to plaintext when server prekey
    /// endpoints are unprovisioned or the peer has not enrolled!
    public func sendEncryptedMessageFailClosed(
        chatId: Int64,
        recipientUserId: UInt64?,
        memberUserIds: [UInt64] = [],
        isGroup: Bool,
        plaintext: String,
        router: ShellNavigationRouter? = nil,
        isCompact: Bool = true,
        locale: FabushiLocale = .en
    ) async -> Result<FabushiSendMessageResponse, Error> {
        do {
            let response: FabushiSendMessageResponse
            if isGroup {
                response = try await client.e2eeSendEncryptedGroup(
                    chatId: chatId,
                    memberUserIds: memberUserIds,
                    plaintext: plaintext
                )
            } else {
                guard let recipientUserId else {
                    throw FabushiHostError.invalidArguments(message: "Missing recipient user ID")
                }
                response = try await client.e2eeSendEncryptedDm(
                    chatId: chatId,
                    recipientUserId: recipientUserId,
                    plaintext: plaintext
                )
            }
            return .success(response)
        } catch {
            _ = router?.showChatToastIfVisible(
                FabushiMessagingStrings.localized(.e2eePeerNotEnrolledNotice, locale: locale),
                chatId: chatId,
                coordinator: coordinator,
                isCompact: isCompact,
                isError: true
            )
            return .failure(error)
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
