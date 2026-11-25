import Foundation

// MARK: - Typed Swift 6 Messaging RPC Client (All Verified Rust Host RPCs)

/// Generation-fenced typed RPC wrapper over `FabushiCoordinator.dispatchFenced` / `executeFenced`
/// matching every verified handler in `source/host/src/handlers_{messaging,agent,platform,core}.rs`.
@MainActor
public final class FabushiMessagingHostClient {
    public let coordinator: FabushiCoordinator

    public init(coordinator: FabushiCoordinator) {
        self.coordinator = coordinator
    }

    // MARK: - Private Nullable Decoding Helper

    /// Decodes an RPC whose `result` may legitimately be JSON `null` (e.g. `chats.draftGet`, `outbox.get`)
    /// without tripping `FabushiRPCResponse<T?>` nil-unwrapping in `FabushiRustHostActor`.
    private func dispatchNullable<Req: Encodable & Sendable, Res: Decodable & Sendable>(
        method: String,
        params: Req
    ) async throws -> Res? {
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(method: method, params: params)
        if case .null = raw {
            return nil
        }
        let encoded = try JSONEncoder().encode(raw)
        return try JSONDecoder().decode(Res.self, from: encoded)
    }

    private func decodeJSONValue<Res: Decodable>(_ raw: FabushiJSONValue) throws -> Res {
        let data = try JSONEncoder().encode(raw)
        return try JSONDecoder().decode(Res.self, from: data)
    }

    // MARK: - 1. `chats.*`

    public func listChats(localOnly: Bool = false, fallbackLocal: Bool = false) async throws -> [FabushiMessagingChat] {
        struct Params: Encodable, Sendable {
            let local_only: Bool
            let fallback_local: Bool
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("list"),
            params: Params(local_only: localOnly, fallback_local: fallbackLocal)
        )
    }

    public func getChat(chatId: Int64) async throws -> FabushiMessagingChat? {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await dispatchNullable(
            method: FabushiRPCNamespace.chats.method("get"),
            params: Params(chat_id: chatId)
        )
    }

    public func createChat(_ request: FabushiCreateChatRequest) async throws -> FabushiMessagingChat {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("create"),
            params: request
        )
    }

    public func provisionWelcomeChat() async throws -> FabushiMessagingChat {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("provisionWelcome"),
            params: FabushiEmptyParams()
        )
    }

    public func updateChatInfo(chatId: Int64, title: String? = nil, about: String? = nil) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let title: String?
            let about: String?
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("updateInfo"),
            params: Params(chat_id: chatId, title: title, about: about)
        )
        return raw["updated"]?.boolValue ?? true
    }

    public func deleteChat(chatId: Int64) async throws {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("delete"),
            params: Params(chat_id: chatId)
        )
    }

    public func clearChatHistory(chatId: Int64) async throws {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("clearHistory"),
            params: Params(chat_id: chatId)
        )
    }

    public func leaveChat(chatId: Int64) async throws {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("leave"),
            params: Params(chat_id: chatId)
        )
    }

    public func setChatTTL(chatId: Int64, ttlSeconds: UInt32) async throws -> UInt32 {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let ttl_seconds: UInt32
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("setTtl"),
            params: Params(chat_id: chatId, ttl_seconds: ttlSeconds)
        )
        return UInt32(raw["ttl_seconds"]?.uint64Value ?? UInt64(ttlSeconds))
    }

    public func setChatArchived(chatId: Int64, archived: Bool) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let archived: Bool
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("setArchived"),
            params: Params(chat_id: chatId, archived: archived)
        )
    }

    public func setChatPinned(chatId: Int64, pinned: Bool) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let pinned: Bool
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("setPinned"),
            params: Params(chat_id: chatId, pinned: pinned)
        )
    }

    public func setChatMuted(chatId: Int64, muted: Bool, muteUntilMs: Int64? = nil) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let muted: Bool
            let mute_until_ms: Int64?
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("setMuted"),
            params: Params(chat_id: chatId, muted: muted, mute_until_ms: muteUntilMs)
        )
    }

    public func listMembers(chatId: Int64) async throws -> [FabushiChatMember] {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("membersList"),
            params: Params(chat_id: chatId)
        )
    }

    public func addMembers(chatId: Int64, userIds: [UInt64]) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let user_ids: [UInt64]
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("membersAdd"),
            params: Params(chat_id: chatId, user_ids: userIds)
        )
        return raw["added"]?.boolValue ?? true
    }

    public func updateMemberRole(
        chatId: Int64,
        userId: UInt64,
        role: String,
        adminTitle: String? = nil,
        adminRights: UInt64? = nil
    ) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let user_id: UInt64
            let role: String
            let admin_title: String?
            let admin_rights: UInt64?
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("memberRoleUpdate"),
            params: Params(
                chat_id: chatId,
                user_id: userId,
                role: role,
                admin_title: adminTitle,
                admin_rights: adminRights
            )
        )
        return raw["updated"]?.boolValue ?? true
    }

    public func removeMember(chatId: Int64, userId: UInt64) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let user_id: UInt64
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("memberRemove"),
            params: Params(chat_id: chatId, user_id: userId)
        )
        return raw["removed"]?.boolValue ?? true
    }

    public func listInvites(chatId: Int64) async throws -> [FabushiInviteLink] {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("invitesList"),
            params: Params(chat_id: chatId)
        )
    }

    public func createInvite(
        chatId: Int64,
        title: String? = nil,
        usageLimit: UInt32? = nil,
        expireDateMs: Int64? = nil,
        requestNeeded: Bool = false
    ) async throws -> FabushiInviteLink {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let title: String?
            let usage_limit: UInt32?
            let expire_date_ms: Int64?
            let request_needed: Bool
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("inviteCreate"),
            params: Params(
                chat_id: chatId,
                title: title,
                usage_limit: usageLimit,
                expire_date_ms: expireDateMs,
                request_needed: requestNeeded
            )
        )
    }

    public func revokeInvite(chatId: Int64, inviteId: String) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let invite_id: String
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("inviteRevoke"),
            params: Params(chat_id: chatId, invite_id: inviteId)
        )
    }

    public func joinInvite(code: String) async throws -> FabushiMessagingChat {
        struct Params: Encodable, Sendable { let code: String }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("inviteJoin"),
            params: Params(code: code)
        )
    }

    public func listPinnedMessages(chatId: Int64) async throws -> [FabushiPinnedMessageRecord] {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("pinsList"),
            params: Params(chat_id: chatId)
        )
    }

    public func pinMessage(chatId: Int64, messageId: Int64) async throws -> FabushiPinnedMessageRecord {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let message_id: Int64
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("pinMessage"),
            params: Params(chat_id: chatId, message_id: messageId)
        )
    }

    public func unpinMessage(chatId: Int64, messageId: Int64) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let message_id: Int64
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("unpinMessage"),
            params: Params(chat_id: chatId, message_id: messageId)
        )
    }

    public func getDraft(chatId: Int64) async throws -> FabushiDraftRecord? {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await dispatchNullable(
            method: FabushiRPCNamespace.chats.method("draftGet"),
            params: Params(chat_id: chatId)
        )
    }

    public func saveDraft(_ draft: FabushiDraftRecord) async throws -> FabushiDraftRecord {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let draft: FabushiDraftRecord
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("draftSave"),
            params: Params(chat_id: draft.chatId, draft: draft)
        )
    }

    public func clearDraft(chatId: Int64) async throws {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("draftClear"),
            params: Params(chat_id: chatId)
        )
    }

    public func listSendAsPeers(chatId: Int64) async throws -> [FabushiSendAsPeer] {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("sendAsPeers"),
            params: Params(chat_id: chatId)
        )
    }

    public func getChannelStatistics(chatId: Int64) async throws -> FabushiChannelStatistics {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("statistics"),
            params: Params(chat_id: chatId)
        )
    }

    public func exportChatHistory(chatId: Int64, format: String = "json") async throws -> String {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let format: String
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("export"),
            params: Params(chat_id: chatId, format: format)
        )
        return raw["export"]?.stringValue ?? raw.toJSONString()
    }

    @discardableResult
    public func subscribeWs(chatId: Int64) async throws -> Bool {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("subscribeWs"),
            params: Params(chat_id: chatId)
        )
        return raw["sent"]?.boolValue ?? raw["subscribed"]?.boolValue ?? false
    }

    @discardableResult
    public func unsubscribeWs(chatId: Int64) async throws -> Bool {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.chats.method("unsubscribeWs"),
            params: Params(chat_id: chatId)
        )
        return raw["sent"]?.boolValue ?? raw["unsubscribed"]?.boolValue ?? false
    }

    // MARK: - 2. `folders.*`

    public func listFolders() async throws -> [FabushiFolder] {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.folders.method("list"),
            params: FabushiEmptyParams()
        )
    }

    public func saveFolder(_ folder: FabushiFolder) async throws -> FabushiFolder {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.folders.method("save"),
            params: folder
        )
    }

    public func updateFolder(folderId: Int64, folder: FabushiFolder) async throws -> FabushiFolder {
        struct Params: Encodable, Sendable {
            let folder_id: Int64
            let folder: FabushiFolder
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.folders.method("update"),
            params: Params(folder_id: folderId, folder: folder)
        )
    }

    public func deleteFolder(folderId: Int64) async throws {
        struct Params: Encodable, Sendable { let folder_id: Int64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.folders.method("delete"),
            params: Params(folder_id: folderId)
        )
    }

    // MARK: - 3. `topics.*`

    public func listTopics(chatId: Int64) async throws -> [FabushiForumTopic] {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.topics.method("list"),
            params: Params(chat_id: chatId)
        )
    }

    public func createTopic(
        chatId: Int64,
        title: String,
        iconColor: Int32 = 0,
        iconEmojiId: Int64? = nil
    ) async throws -> FabushiForumTopic {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let title: String
            let icon_color: Int32
            let icon_emoji_id: Int64?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.topics.method("create"),
            params: Params(chat_id: chatId, title: title, icon_color: iconColor, icon_emoji_id: iconEmojiId)
        )
    }

    public func updateTopic(
        chatId: Int64,
        topicId: Int64,
        title: String? = nil,
        isPinned: Bool? = nil,
        isClosed: Bool? = nil
    ) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let topic_id: Int64
            let title: String?
            let is_pinned: Bool?
            let is_closed: Bool?
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.topics.method("update"),
            params: Params(chat_id: chatId, topic_id: topicId, title: title, is_pinned: isPinned, is_closed: isClosed)
        )
        return raw["updated"]?.boolValue ?? true
    }

    public func deleteTopic(chatId: Int64, topicId: Int64) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let topic_id: Int64
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.topics.method("delete"),
            params: Params(chat_id: chatId, topic_id: topicId)
        )
        return raw["deleted"]?.boolValue ?? true
    }

    // MARK: - 4. `contacts.*`

    public func listContacts() async throws -> [FabushiContact] {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("list"),
            params: FabushiEmptyParams()
        )
    }

    public func addContact(_ request: FabushiAddContactRequest) async throws -> FabushiAddContactResponse {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("add"),
            params: request
        )
    }

    public func deleteContact(userId: UInt64) async throws {
        struct Params: Encodable, Sendable { let user_id: UInt64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("delete"),
            params: Params(user_id: userId)
        )
    }

    public func resolvePeer(username: String) async throws -> FabushiUserProfile {
        struct Params: Encodable, Sendable { let username: String }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("resolvePeer"),
            params: Params(username: username)
        )
    }

    public func getUserProfile(userId: UInt64) async throws -> FabushiUserProfile {
        struct Params: Encodable, Sendable { let user_id: UInt64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("userProfile"),
            params: Params(user_id: userId)
        )
    }

    public func getMyProfile() async throws -> FabushiUserProfile {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("myProfile"),
            params: FabushiEmptyParams()
        )
    }

    public func listBlockedUsers() async throws -> [FabushiBlockedUser] {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("blockedList"),
            params: FabushiEmptyParams()
        )
    }

    public func blockUser(userId: UInt64) async throws -> FabushiBlockedUser {
        struct Params: Encodable, Sendable { let user_id: UInt64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("block"),
            params: Params(user_id: userId)
        )
    }

    public func unblockUser(userId: UInt64) async throws {
        struct Params: Encodable, Sendable { let user_id: UInt64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("unblock"),
            params: Params(user_id: userId)
        )
    }

    // MARK: - 5. `messages.*` (ALWAYS uses `messages.sendDurable` per TRAP #1 / `D14-03`!)

    /// Primary outgoing send RPC forwarding full `RichSendMessageRequest` including BOTH `reply_to`
    /// and `quote_text`, `content`, `entities`, `topic_id`, `client_nonce`, `forward_from_*`,
    /// `silent`, `scheduled_at_ms`, `inline_keyboard`, `send_as_peer_id`, and `skip_server_bot_reply`.
    public func sendDurable(_ request: FabushiRichSendMessageRequest) async throws -> FabushiSendMessageResponse {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("sendDurable"),
            params: request
        )
    }

    public func listLocalMessages(chatId: Int64) async throws -> [FabushiMessageRecord] {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("localList"),
            params: Params(chat_id: chatId)
        )
    }

    public func fetchHistory(chatId: Int64, limit: Int = 50) async throws -> [FabushiMessageRecord] {
        guard chatId >= 0 else {
            return try await listLocalMessages(chatId: chatId)
        }
        struct Params: Encodable, Sendable {
            let chat_id: UInt64
            let limit: Int
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("history"),
            params: Params(chat_id: UInt64(chatId), limit: limit)
        )
    }

    public func editMessage(
        chatId: Int64,
        messageId: Int64,
        text: String,
        entities: [FabushiMessageEntity] = []
    ) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let message_id: Int64
            let text: String
            let entities: [FabushiMessageEntity]
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("edit"),
            params: Params(chat_id: chatId, message_id: messageId, text: text, entities: entities)
        )
    }

    public func deleteMessage(chatId: Int64, messageId: Int64, revoke: Bool = true) async throws {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let message_id: Int64
            let revoke: Bool
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("delete"),
            params: Params(chat_id: chatId, message_id: messageId, revoke: revoke)
        )
    }

    public func reactToMessage(
        chatId: Int64,
        messageId: Int64,
        emoji: String,
        action: String? = "toggle"
    ) async throws -> [FabushiMessageReaction] {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let message_id: Int64
            let emoji: String
            let action: String?
        }
        struct Response: Decodable, Sendable {
            let reactions: [FabushiMessageReaction]?
        }
        let resp: Response = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("react"),
            params: Params(chat_id: chatId, message_id: messageId, emoji: emoji, action: action)
        )
        return resp.reactions ?? []
    }

    public func markRead(chatId: Int64, maxId: Int64, topicId: Int64? = nil) async throws -> Int {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let max_id: Int64
            let topic_id: Int64?
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("markRead"),
            params: Params(chat_id: chatId, max_id: maxId, topic_id: topicId)
        )
        return Int(raw["unread_count"]?.int64Value ?? 0)
    }

    public func createPoll(
        chatId: Int64,
        question: String,
        options: [String],
        isAnonymous: Bool = true,
        multipleChoice: Bool = false,
        isQuiz: Bool = false,
        correctOptionId: UInt8? = nil,
        solution: String? = nil,
        topicId: Int64? = nil,
        clientNonce: UInt64? = nil
    ) async throws -> FabushiPollCard {
        struct PollPayload: Encodable, Sendable {
            let question: String
            let options: [String]
            let is_anonymous: Bool
            let multiple_choice: Bool
            let is_quiz: Bool
            let correct_option_id: UInt8?
            let solution: String?
            let topic_id: Int64?
            let client_nonce: UInt64?
        }
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let poll: PollPayload
        }
        struct Resp: Decodable, Sendable {
            let poll: FabushiPollCard
        }
        let resp: Resp = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("pollCreate"),
            params: Params(
                chat_id: chatId,
                poll: PollPayload(
                    question: question,
                    options: options,
                    is_anonymous: isAnonymous,
                    multiple_choice: multipleChoice,
                    is_quiz: isQuiz,
                    correct_option_id: correctOptionId,
                    solution: solution,
                    topic_id: topicId,
                    client_nonce: clientNonce
                )
            )
        )
        return resp.poll
    }

    public func votePoll(pollId: UInt64, optionIds: [UInt8]) async throws -> FabushiPollCard {
        struct Params: Encodable, Sendable {
            let poll_id: UInt64
            let option_ids: [UInt8]
        }
        struct Resp: Decodable, Sendable { let poll: FabushiPollCard }
        let resp: Resp = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("pollVote"),
            params: Params(poll_id: pollId, option_ids: optionIds)
        )
        return resp.poll
    }

    public func closePoll(pollId: UInt64) async throws -> FabushiPollCard {
        struct Params: Encodable, Sendable { let poll_id: UInt64 }
        struct Resp: Decodable, Sendable { let poll: FabushiPollCard }
        let resp: Resp = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("pollClose"),
            params: Params(poll_id: pollId)
        )
        return resp.poll
    }

    public func sendTyping(chatId: Int64, action: String = "typing") async throws -> Bool {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let action: String
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.messages.method("sendTyping"),
            params: Params(chat_id: chatId, action: action)
        )
        return raw["sent"]?.boolValue ?? false
    }

    // MARK: - 6. `sync.*` & `outbox.*`

    public func lastPts(chatId: Int64) async throws -> UInt64 {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.sync.method("lastPts"),
            params: Params(chat_id: chatId)
        )
        return raw["pts"]?.uint64Value ?? 0
    }

    public func setPts(chatId: Int64, pts: UInt64) async throws -> UInt64 {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let pts: UInt64
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.sync.method("setPts"),
            params: Params(chat_id: chatId, pts: pts)
        )
        return raw["pts"]?.uint64Value ?? pts
    }

    public func pendingSyncGapChats() async throws -> [Int64] {
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.sync.method("pendingGaps"),
            params: FabushiEmptyParams()
        )
        return raw["gap_chats"]?.arrayValue?.compactMap(\.int64Value) ?? []
    }

    public func getSyncDifference(chatId: Int64, limit: UInt32 = 100, pts: UInt64? = nil) async throws -> FabushiSyncDifferenceResponse {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let limit: UInt32
            let pts: UInt64?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.sync.method("getDifference"),
            params: Params(chat_id: chatId, limit: limit, pts: pts)
        )
    }

    public func syncAllGaps(limit: UInt32 = 100) async throws -> [FabushiSyncDifferenceResponse] {
        struct Params: Encodable, Sendable { let limit: UInt32 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.sync.method("syncAllGaps"),
            params: Params(limit: limit)
        )
    }

    public func evaluatePtsUpdate(
        chatId: Int64,
        pts: UInt64,
        ptsCount: UInt32 = 1,
        event: FabushiJSONValue,
        timestampMs: Int64 = FabushiWireDateParser.nowMs()
    ) async throws -> FabushiPtsEvaluationResult {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let pts: UInt64
            let pts_count: UInt32
            let event: FabushiJSONValue
            let timestamp_ms: Int64
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.sync.method("evaluatePtsUpdate"),
            params: Params(
                chat_id: chatId,
                pts: pts,
                pts_count: ptsCount,
                event: event,
                timestamp_ms: timestampMs
            )
        )
    }

    public func enqueueOutbox(_ request: FabushiRichSendMessageRequest) async throws -> FabushiOutboxEntry {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.outbox.method("enqueue"),
            params: request
        )
    }

    public func listOutbox(status: String? = nil) async throws -> [FabushiOutboxEntry] {
        struct Params: Encodable, Sendable { let status: String? }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.outbox.method("list"),
            params: Params(status: status)
        )
    }

    public func getOutboxEntry(clientNonce: UInt64) async throws -> FabushiOutboxEntry? {
        struct Params: Encodable, Sendable { let client_nonce: UInt64 }
        return try await dispatchNullable(
            method: FabushiRPCNamespace.outbox.method("get"),
            params: Params(client_nonce: clientNonce)
        )
    }

    public func outboxPendingCount() async throws -> FabushiOutboxPendingCountResult {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.outbox.method("pendingCount"),
            params: FabushiEmptyParams()
        )
    }

    @discardableResult
    public func retryOutboxNow() async throws -> Int {
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.outbox.method("retryNow"),
            params: FabushiEmptyParams()
        )
        return Int(raw["reset_count"]?.int64Value ?? 0)
    }

    @discardableResult
    public func recoverInFlightOutbox() async throws -> Int {
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.outbox.method("recoverInFlight"),
            params: FabushiEmptyParams()
        )
        return Int(raw["recovered"]?.int64Value ?? 0)
    }

    // MARK: - 7. `search.*` (Strictly Backed Scopes — ZERO `community.*` RPCs per `BLK-08`)

    public func searchRemoteMessages(
        query: String,
        filter: String? = nil,
        chatId: Int64? = nil,
        limit: Int = 50
    ) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let query: String
            let filter: String?
            let chat_id: Int64?
            let limit: Int
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.search.method("messages"),
            params: Params(query: query, filter: filter, chat_id: chatId, limit: limit)
        )
    }

    public func searchGlobalCatalog(query: String, category: String? = nil) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let query: String
            let category: String?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.search.method("globalCatalog"),
            params: Params(query: query, category: category)
        )
    }

    public func indexLocalSearchMessage(
        messageId: Int64,
        chatId: Int64,
        senderId: UInt64,
        timestampMs: Int64,
        body: String
    ) async throws {
        struct Params: Encodable, Sendable {
            let message_id: Int64
            let chat_id: Int64
            let sender_id: UInt64
            let timestamp_ms: Int64
            let body: String
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.search.method("indexMessage"),
            params: Params(
                message_id: messageId,
                chat_id: chatId,
                sender_id: senderId,
                timestamp_ms: timestampMs,
                body: body
            )
        )
    }

    public func searchLocalMessages(
        query: String,
        chatId: Int64? = nil,
        limit: Int = 50
    ) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let query: String
            let chat_id: Int64?
            let limit: Int
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.search.method("localMessages"),
            params: Params(query: query, chat_id: chatId, limit: limit)
        )
    }

    public func searchSessions(query: String, limit: Int = 20) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let query: String
            let limit: Int
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.search.method("sessions"),
            params: Params(query: query, limit: limit)
        )
    }

    // MARK: - 8. `e2ee.*`

    public func e2eeIdentity() async throws -> FabushiE2EEIdentity {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("identity"),
            params: FabushiEmptyParams()
        )
    }

    public func e2eeSafetyFingerprint(
        localIdentityKeyB64: String? = nil,
        remoteIdentityKeyB64: String
    ) async throws -> String {
        struct Params: Encodable, Sendable {
            let local_identity_key_b64: String?
            let remote_identity_key_b64: String
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("safetyFingerprint"),
            params: Params(
                local_identity_key_b64: localIdentityKeyB64,
                remote_identity_key_b64: remoteIdentityKeyB64
            )
        )
        return raw["emojis"]?.stringValue ?? ""
    }

    public func e2eeInitAndPublishPrekeys(otpkBatchSize: Int = 20) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable { let otpk_batch_size: Int }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("initAndPublishPrekeys"),
            params: Params(otpk_batch_size: otpkBatchSize)
        )
    }

    public func e2eeSendEncryptedDm(
        chatId: Int64,
        recipientUserId: UInt64,
        plaintext: String
    ) async throws -> FabushiSendMessageResponse {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let recipient_user_id: UInt64
            let plaintext: String
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("sendEncryptedDm"),
            params: Params(chat_id: chatId, recipient_user_id: recipientUserId, plaintext: plaintext)
        )
    }

    public func e2eeSendEncryptedGroup(
        chatId: Int64,
        memberUserIds: [UInt64],
        plaintext: String
    ) async throws -> FabushiSendMessageResponse {
        struct Params: Encodable, Sendable {
            let chat_id: Int64
            let member_user_ids: [UInt64]
            let plaintext: String
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("sendEncryptedGroup"),
            params: Params(chat_id: chatId, member_user_ids: memberUserIds, plaintext: plaintext)
        )
    }

    public func e2eeRatchetInitPair(
        aliceSessionId: String,
        bobSessionId: String,
        associatedData: String = "fabushi-dr-v1"
    ) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let alice_session_id: String
            let bob_session_id: String
            let associated_data: String
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("ratchetInitPair"),
            params: Params(
                alice_session_id: aliceSessionId,
                bob_session_id: bobSessionId,
                associated_data: associatedData
            )
        )
    }

    public func e2eeRatchetEncrypt(
        sessionId: String,
        plaintext: String
    ) async throws -> (header: FabushiJSONValue, ciphertextB64: String) {
        struct Params: Encodable, Sendable {
            let session_id: String
            let plaintext: String
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("ratchetEncrypt"),
            params: Params(session_id: sessionId, plaintext: plaintext)
        )
        guard let header = raw["header"],
              let ciphertextB64 = raw["ciphertext_b64"]?.stringValue
        else {
            throw FabushiHostError.invalidResponse(raw: "Missing header/ciphertext_b64 in e2ee.ratchetEncrypt")
        }
        return (header, ciphertextB64)
    }

    public func e2eeRatchetDecrypt(
        sessionId: String,
        header: FabushiJSONValue,
        ciphertextB64: String
    ) async throws -> String {
        struct Params: Encodable, Sendable {
            let session_id: String
            let header: FabushiJSONValue
            let ciphertext_b64: String
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("ratchetDecrypt"),
            params: Params(session_id: sessionId, header: header, ciphertext_b64: ciphertextB64)
        )
        return raw["plaintext_utf8"]?.stringValue ?? ""
    }

    public func e2eeSenderKeyCreate(
        keyId: String,
        chatId: Int64,
        senderUserId: UInt64,
        senderDeviceId: String
    ) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let key_id: String
            let chat_id: Int64
            let sender_user_id: UInt64
            let sender_device_id: String
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("senderKeyCreate"),
            params: Params(
                key_id: keyId,
                chat_id: chatId,
                sender_user_id: senderUserId,
                sender_device_id: senderDeviceId
            )
        )
        guard let dist = raw["distribution"] else {
            throw FabushiHostError.invalidResponse(raw: "Missing distribution in e2ee.senderKeyCreate")
        }
        return dist
    }

    public func e2eeSenderKeyImportDistribution(
        keyId: String,
        distribution: FabushiJSONValue
    ) async throws {
        struct Params: Encodable, Sendable {
            let key_id: String
            let distribution: FabushiJSONValue
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("senderKeyImportDistribution"),
            params: Params(key_id: keyId, distribution: distribution)
        )
    }

    public func e2eeSenderKeyEncrypt(keyId: String, plaintext: String) async throws -> FabushiJSONValue {
        struct Params: Encodable, Sendable {
            let key_id: String
            let plaintext: String
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("senderKeyEncrypt"),
            params: Params(key_id: keyId, plaintext: plaintext)
        )
    }

    public func e2eeSenderKeyDecrypt(keyId: String, ciphertext: FabushiJSONValue) async throws -> String {
        struct Params: Encodable, Sendable {
            let key_id: String
            let ciphertext: FabushiJSONValue
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.e2ee.method("senderKeyDecrypt"),
            params: Params(key_id: keyId, ciphertext: ciphertext)
        )
        return raw["plaintext_utf8"]?.stringValue ?? ""
    }

    // MARK: - 9. `media.*`, `stories.*`, `stickers.*`

    public func uploadMedia(filename: String, mimeType: String, dataB64: String) async throws -> FabushiMediaUploadResponse {
        struct Params: Encodable, Sendable {
            let filename: String
            let mime_type: String
            let data_b64: String
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("upload"),
            params: Params(filename: filename, mime_type: mimeType, data_b64: dataB64)
        )
    }

    public func downloadMedia(fileId: String) async throws -> Data {
        struct Params: Encodable, Sendable { let file_id: String }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("download"),
            params: Params(file_id: fileId)
        )
        guard let b64 = raw["data_b64"]?.stringValue,
              let data = Data(base64Encoded: b64)
        else {
            throw FabushiHostError.invalidResponse(raw: "Missing data_b64 in media.download")
        }
        return data
    }

    public func downloadMediaRange(
        fileId: String,
        start: UInt64,
        endInclusive: UInt64? = nil
    ) async throws -> FabushiMediaRangeResponse {
        struct Params: Encodable, Sendable {
            let file_id: String
            let start: UInt64
            let end_inclusive: UInt64?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("downloadRange"),
            params: Params(file_id: fileId, start: start, end_inclusive: endInclusive)
        )
    }

    public func casGet(fileId: String) async throws -> Data? {
        struct Params: Encodable, Sendable { let file_id: String }
        do {
            let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.media.method("casGet"),
                params: Params(file_id: fileId)
            )
            guard let b64 = raw["data_b64"]?.stringValue else { return nil }
            return Data(base64Encoded: b64)
        } catch let error as FabushiHostError {
            if case .notFound = error { return nil }
            throw error
        }
    }

    public func casPut(fileId: String, data: Data, mimeType: String? = nil) async throws {
        struct Params: Encodable, Sendable {
            let file_id: String
            let data_b64: String
            let mime_type: String?
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("casPut"),
            params: Params(file_id: fileId, data_b64: data.base64EncodedString(), mime_type: mimeType)
        )
    }

    public func localCasPath(fileId: String) async throws -> String? {
        struct Params: Encodable, Sendable { let file_id: String }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("localCasPath"),
            params: Params(file_id: fileId)
        )
        return raw["local_path"]?.stringValue
    }

    public func encodeWaveform(bars: [UInt8]) async throws -> (bytes: [UInt8], base64: String) {
        struct Params: Encodable, Sendable { let bars: [UInt8] }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("encodeWaveform"),
            params: Params(bars: bars)
        )
        let bytes = raw["waveform_bytes"]?.arrayValue?.compactMap { $0.uint64Value.map { UInt8(clamping: $0) } } ?? []
        let b64 = raw["waveform_b64"]?.stringValue ?? ""
        return (bytes, b64)
    }

    public func decodeWaveform(waveformB64: String, targetBars: Int = 100) async throws -> [UInt8] {
        struct Params: Encodable, Sendable {
            let waveform_b64: String
            let target_bars: Int
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.media.method("decodeWaveform"),
            params: Params(waveform_b64: waveformB64, target_bars: targetBars)
        )
        return raw["bars"]?.arrayValue?.compactMap { $0.uint64Value.map { UInt8(clamping: $0) } } ?? []
    }

    public func listStories() async throws -> [FabushiStoryItem] {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.stories.method("list"),
            params: FabushiEmptyParams()
        )
    }

    public func createStory(
        mediaId: String? = nil,
        mediaType: String = "photo",
        caption: String? = nil,
        privacy: String = "everyone"
    ) async throws -> FabushiStoryItem {
        struct Params: Encodable, Sendable {
            let media_id: String?
            let media_type: String
            let caption: String?
            let privacy: String
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.stories.method("create"),
            params: Params(media_id: mediaId, media_type: mediaType, caption: caption, privacy: privacy)
        )
    }

    public func deleteStory(storyId: String) async throws {
        struct Params: Encodable, Sendable { let story_id: String }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.stories.method("delete"),
            params: Params(story_id: storyId)
        )
    }

    public func listStickerSets(query: String? = nil) async throws -> [FabushiStickerPack] {
        struct Params: Encodable, Sendable { let query: String? }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.stickers.method("listSets"),
            params: Params(query: query)
        )
    }

    public func installStickerSet(setId: String, installed: Bool = true) async throws {
        struct Params: Encodable, Sendable {
            let set_id: String
            let installed: Bool
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.stickers.method("installSet"),
            params: Params(set_id: setId, installed: installed)
        )
    }

    public func searchGifs(query: String, limit: Int = 20) async throws -> [FabushiGifItem] {
        struct Params: Encodable, Sendable {
            let query: String
            let limit: Int
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.stickers.method("searchGifs"),
            params: Params(query: query, limit: limit)
        )
    }

    // MARK: - 10. `fabushi.*`, `magpie.*`, `computers.*`, `miniapp.*`, `session.*`, `multi_agent.*`

    public func listSlashCommands() async throws -> [FabushiSlashCommand] {
        struct Resp: Decodable, Sendable { let commands: [FabushiSlashCommand] }
        let resp: Resp = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("listCommands"),
            params: FabushiEmptyParams()
        )
        return resp.commands
    }

    public func autocompleteSlashCommands(prefix: String = "/") async throws -> [FabushiSlashCommand] {
        struct Params: Encodable, Sendable { let prefix: String }
        struct Resp: Decodable, Sendable { let commands: [FabushiSlashCommand] }
        let resp: Resp = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("autocompleteCommands"),
            params: Params(prefix: prefix)
        )
        return resp.commands
    }

    public func runSlashCommand(
        command: String,
        sessionId: String? = nil,
        chatId: Int64? = nil,
        botId: UInt64? = nil
    ) async throws -> FabushiCommandExecutionResponse {
        struct Params: Encodable, Sendable {
            let command: String
            let session_id: String?
            let chat_id: Int64?
            let bot_id: UInt64?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("runCommand"),
            params: Params(command: command, session_id: sessionId, chat_id: chatId, bot_id: botId)
        )
    }

    public func listApprovals(sessionId: String? = nil, pendingOnly: Bool = true) async throws -> [FabushiApprovalItem] {
        struct Params: Encodable, Sendable {
            let session_id: String?
            let pending_only: Bool
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("approvalsList"),
            params: Params(session_id: sessionId, pending_only: pendingOnly)
        )
    }

    public func decideApproval(
        approvalId: String,
        status: String,
        decidedBy: String? = nil,
        remote: Bool = false
    ) async throws {
        struct Params: Encodable, Sendable {
            let approval_id: String
            let status: String
            let decision: String
            let decided_by: String?
            let remote: Bool
        }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("approvalDecide"),
            params: Params(
                approval_id: approvalId,
                status: status,
                decision: status,
                decided_by: decidedBy,
                remote: remote
            )
        )
    }

    public func listBots() async throws -> [FabushiBotConfig] {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("botsList"),
            params: FabushiEmptyParams()
        )
    }

    public func getBot(botId: UInt64) async throws -> FabushiBotConfig {
        struct Params: Encodable, Sendable { let bot_id: UInt64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("botGet"),
            params: Params(bot_id: botId)
        )
    }

    public func patchBot(
        botId: UInt64,
        displayName: String? = nil,
        descriptionText: String? = nil,
        preferredModel: String? = nil,
        thinkingLevel: String? = nil,
        approvalMode: String? = nil,
        toolsetPreset: String? = nil,
        notifyOnAgentUpdates: Bool? = nil,
        hiddenFromSidebar: Bool? = nil
    ) async throws -> FabushiBotConfig {
        struct Params: Encodable, Sendable {
            let bot_id: UInt64
            let display_name: String?
            let description: String?
            let preferred_model: String?
            let thinking_level: String?
            let approval_mode: String?
            let toolset_preset: String?
            let notify_on_agent_updates: Bool?
            let hidden_from_sidebar: Bool?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.fabushi.method("botPatch"),
            params: Params(
                bot_id: botId,
                display_name: displayName,
                description: descriptionText,
                preferred_model: preferredModel,
                thinking_level: thinkingLevel,
                approval_mode: approvalMode,
                toolset_preset: toolsetPreset,
                notify_on_agent_updates: notifyOnAgentUpdates,
                hidden_from_sidebar: hiddenFromSidebar
            )
        )
    }

    public func fetchMagpieState(remote: Bool = true) async throws -> FabushiAiModelCatalog {
        struct Params: Encodable, Sendable { let remote: Bool }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.magpie.method("state"),
            params: Params(remote: remote)
        )
        return FabushiAiModelCatalog.fromMagpieStateJSON(raw)
    }

    public func listMagpiePresets() async throws -> FabushiJSONValue {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.magpie.method("listPresets"),
            params: FabushiEmptyParams()
        )
    }

    public func listMagpieGroups() async throws -> [FabushiJSONValue] {
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.magpie.method("listGroups"),
            params: FabushiEmptyParams()
        )
        return raw.arrayValue ?? []
    }

    /// `D10-09`: Fetches the live `FabushiAiModelCatalog` strictly from `magpie.state(remote: true)`
    /// with offline fallback to local `magpie.state(remote: false)` + `magpie.listGroups`.
    /// ZERO hardcoded model defaults.
    public func fetchEffectiveAiModelCatalog() async -> FabushiAiModelCatalog {
        if let remoteCatalog = try? await fetchMagpieState(remote: true),
           remoteCatalog.hasConfiguredProvider || !remoteCatalog.routingGroups.isEmpty {
            return remoteCatalog
        }
        if let localCatalog = try? await fetchMagpieState(remote: false) {
            if !localCatalog.routingGroups.isEmpty || localCatalog.hasConfiguredProvider {
                return localCatalog
            }
        }
        if let groups = try? await listMagpieGroups() {
            let names = groups.compactMap { $0["name"]?.stringValue ?? $0["id"]?.stringValue }.filter { !$0.isEmpty }
            return FabushiAiModelCatalog(
                activeProvider: nil,
                activeModel: nil,
                models: [],
                routingGroups: names,
                hasConfiguredProvider: false
            )
        }
        return FabushiAiModelCatalog()
    }

    public func listComputers() async throws -> [FabushiBotComputer] {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.computers.method("list"),
            params: FabushiEmptyParams()
        )
    }

    public func getBotComputerProfile(botId: UInt64) async throws -> FabushiBotComputerProfile {
        struct Params: Encodable, Sendable { let bot_id: UInt64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.computers.method("botProfileGet"),
            params: Params(bot_id: botId)
        )
    }

    public func updateBotComputerProfile(
        botId: UInt64,
        boundNodeId: String? = nil,
        clearBoundNode: Bool = false,
        personaPrompt: String? = nil,
        preferredModel: String? = nil,
        approvalMode: String? = nil,
        allowCrossDeviceControl: Bool? = nil,
        allowCloudSelfConfig: Bool? = nil
    ) async throws -> FabushiBotComputerProfile {
        struct Params: Encodable, Sendable {
            let bot_id: UInt64
            let bound_node_id: String?
            let clear_bound_node: Bool
            let persona_prompt: String?
            let preferred_model: String?
            let approval_mode: String?
            let allow_cross_device_control: Bool?
            let allow_cloud_self_config: Bool?
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.computers.method("botProfileUpdate"),
            params: Params(
                bot_id: botId,
                bound_node_id: boundNodeId,
                clear_bound_node: clearBoundNode,
                persona_prompt: personaPrompt,
                preferred_model: preferredModel,
                approval_mode: approvalMode,
                allow_cross_device_control: allowCrossDeviceControl,
                allow_cloud_self_config: allowCloudSelfConfig
            )
        )
    }

    public func getBotPublicProfile(target: String) async throws -> FabushiBotPublicProfile {
        struct Params: Encodable, Sendable { let target: String }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("botProfile"),
            params: Params(target: target)
        )
    }

    public func canBotSendMessage(botId: UInt64) async throws -> FabushiCanBotSendResponse {
        struct Params: Encodable, Sendable { let bot_id: UInt64 }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("canBotSendMessage"),
            params: Params(bot_id: botId)
        )
    }

    public func allowBotSendMessage(
        botId: UInt64,
        appId: String? = nil,
        webAppName: String? = nil,
        fromAttachmentMenu: Bool = false
    ) async throws -> FabushiAllowBotSendResponse {
        struct Params: Encodable, Sendable {
            let bot_id: UInt64
            let app_id: String?
            let web_app_name: String?
            let from_attachment_menu: Bool
        }
        return try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("allowBotSendMessage"),
            params: Params(
                bot_id: botId,
                app_id: appId,
                web_app_name: webAppName,
                from_attachment_menu: fromAttachmentMenu
            )
        )
    }

    public func setBotBlocked(botId: UInt64, blocked: Bool) async throws -> Bool {
        struct Params: Encodable, Sendable {
            let bot_id: UInt64
            let blocked: Bool
        }
        let raw: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.miniapp.method("setBotBlocked"),
            params: Params(bot_id: botId, blocked: blocked)
        )
        return raw["ok"]?.boolValue ?? true
    }

    public func connectWs() async throws -> FabushiJSONValue {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.session.method("connectWs"),
            params: FabushiEmptyParams()
        )
    }

    public func wsStatus() async throws -> FabushiWsStatusResult {
        try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.session.method("wsStatus"),
            params: FabushiEmptyParams()
        )
    }

    public func preemptFloorForUser(chatId: Int64) async throws {
        struct Params: Encodable, Sendable { let chat_id: Int64 }
        let _: FabushiJSONValue = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.multiAgent.method("floorPreemptUser"),
            params: Params(chat_id: chatId)
        )
    }
}
