import Foundation
import Observation

// MARK: - Timestamp & Flexible Wire Parsing Helpers

public enum FabushiWireDateParser: Sendable {
    public static func parseTimestampMs(from raw: FabushiJSONValue?) -> Int64? {
        guard let raw else { return nil }
        switch raw {
        case .int(let value):
            return normalizeEpochToMs(value)
        case .double(let value):
            return normalizeEpochToMs(Int64(value))
        case .string(let text):
            return parseTimestampMs(fromString: text)
        default:
            return nil
        }
    }

    public static func parseTimestampMs(fromString raw: String) -> Int64? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if let integer = Int64(trimmed) {
            return normalizeEpochToMs(integer)
        }
        let fractionalStyle = Date.ISO8601FormatStyle(includingFractionalSeconds: true)
        if let date = try? fractionalStyle.parse(trimmed) {
            return Int64(date.timeIntervalSince1970 * 1000.0)
        }
        if let date = try? Date.ISO8601FormatStyle().parse(trimmed) {
            return Int64(date.timeIntervalSince1970 * 1000.0)
        }
        return nil
    }

    public static func normalizeEpochToMs(_ value: Int64) -> Int64 {
        guard value > 0 else { return 0 }
        // Heuristic matching Rust server: < 10_000_000_000 indicates Unix seconds.
        if value < 10_000_000_000 {
            return value * 1_000
        }
        return value
    }

    public static func nowMs() -> Int64 {
        Int64(Date().timeIntervalSince1970 * 1000.0)
    }
}

// MARK: - Chat & Dialog Models (Dual Online `ChatSummary`/`ChatDetails` + Offline `ChatDetailDto`)

public struct FabushiMessagingChat: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: Int64 { chatId }

    public let chatId: Int64
    public var title: String
    public var kind: String
    public var chatType: Int
    public var creatorId: UInt64
    public var username: String?
    public var about: String?
    public var isGroup: Bool
    public var isChannel: Bool
    public var isBot: Bool
    public var isForum: Bool
    public var isArchived: Bool
    public var isPinned: Bool
    public var isMuted: Bool
    public var muteUntilMs: Int64?
    public var pts: UInt64
    public var lastMessageId: Int64
    public var lastMessagePreview: String?
    public var lastMessageDateMs: Int64
    public var unreadCount: Int
    public var readInboxMaxId: Int64
    public var readOutboxMaxId: Int64
    public var memberCount: Int
    public var pinnedMessageId: Int64?
    public var avatarColor: UInt32?
    public var ttlSeconds: UInt32
    public var draft: FabushiDraftRecord?

    /// `D14-04` invariant: group chats are NEVER classified as 1:1 bot DMs even if their title
    /// contains `"Bot"` or `"Autonomous Agent"`.
    public static func resolvedIsBot(rawIsBot: Bool?, isGroup: Bool, isChannel: Bool, kind: String?) -> Bool {
        if isGroup || isChannel {
            return false
        }
        if let rawIsBot {
            return rawIsBot
        }
        return kind?.lowercased() == "bot"
    }

    public init(
        chatId: Int64,
        title: String,
        kind: String = "private",
        chatType: Int = 0,
        creatorId: UInt64 = 0,
        username: String? = nil,
        about: String? = nil,
        isGroup: Bool = false,
        isChannel: Bool = false,
        isBot: Bool = false,
        isForum: Bool = false,
        isArchived: Bool = false,
        isPinned: Bool = false,
        isMuted: Bool = false,
        muteUntilMs: Int64? = nil,
        pts: UInt64 = 0,
        lastMessageId: Int64 = 0,
        lastMessagePreview: String? = nil,
        lastMessageDateMs: Int64 = 0,
        unreadCount: Int = 0,
        readInboxMaxId: Int64 = 0,
        readOutboxMaxId: Int64 = 0,
        memberCount: Int = 0,
        pinnedMessageId: Int64? = nil,
        avatarColor: UInt32? = nil,
        ttlSeconds: UInt32 = 0,
        draft: FabushiDraftRecord? = nil
    ) {
        self.chatId = chatId
        self.title = title
        self.kind = kind
        self.chatType = chatType
        self.creatorId = creatorId
        self.username = username
        self.about = about
        self.isGroup = isGroup
        self.isChannel = isChannel
        self.isBot = Self.resolvedIsBot(rawIsBot: isBot, isGroup: isGroup, isChannel: isChannel, kind: kind)
        self.isForum = isForum
        self.isArchived = isArchived
        self.isPinned = isPinned
        self.isMuted = isMuted
        self.muteUntilMs = muteUntilMs
        self.pts = pts
        self.lastMessageId = lastMessageId
        self.lastMessagePreview = lastMessagePreview
        self.lastMessageDateMs = lastMessageDateMs
        self.unreadCount = unreadCount
        self.readInboxMaxId = readInboxMaxId
        self.readOutboxMaxId = readOutboxMaxId
        self.memberCount = memberCount
        self.pinnedMessageId = pinnedMessageId
        self.avatarColor = avatarColor
        self.ttlSeconds = ttlSeconds
        self.draft = draft
    }

    private enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case id
        case title
        case kind
        case chatType = "chat_type"
        case creatorId = "creator_id"
        case username
        case about
        case isGroup = "is_group"
        case isChannel = "is_channel"
        case isBot = "is_bot"
        case isForum = "is_forum"
        case isArchived = "is_archived"
        case isPinned = "is_pinned"
        case isMuted = "is_muted"
        case muteUntilMs = "mute_until_ms"
        case pts
        case lastMessageId = "last_message_id"
        case lastMessage = "last_message"
        case lastMessagePreview = "last_message_preview"
        case lastMessageDateMs = "last_message_date_ms"
        case updatedAt = "updated_at"
        case unreadCount = "unread_count"
        case readInboxMaxId = "read_inbox_max_id"
        case readOutboxMaxId = "read_outbox_max_id"
        case memberCount = "member_count"
        case membersCount = "members_count"
        case pinnedMessageId = "pinned_message_id"
        case avatarColor = "avatar_color"
        case ttlSeconds = "ttl_seconds"
        case draft
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let cid = try? container.decode(Int64.self, forKey: .chatId) {
            self.chatId = cid
        } else if let cidU = try? container.decode(UInt64.self, forKey: .chatId) {
            self.chatId = Int64(bitPattern: cidU)
        } else if let cid = try? container.decode(Int64.self, forKey: .id) {
            self.chatId = cid
        } else {
            self.chatId = 0
        }

        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""

        let rawKind = try container.decodeIfPresent(String.self, forKey: .kind)
        let decodedChatType: Int
        if let intType = try? container.decodeIfPresent(Int.self, forKey: .chatType) {
            decodedChatType = intType
        } else if let strType = try? container.decodeIfPresent(String.self, forKey: .chatType) {
            switch strType.lowercased() {
            case "group", "supergroup": decodedChatType = 1
            case "channel": decodedChatType = 2
            case "bot": decodedChatType = 3
            default: decodedChatType = 0
            }
        } else {
            decodedChatType = 0
        }
        self.chatType = decodedChatType

        let rawIsGroup = try container.decodeIfPresent(Bool.self, forKey: .isGroup)
        let rawIsChannel = try container.decodeIfPresent(Bool.self, forKey: .isChannel)
        let rawIsBot = try container.decodeIfPresent(Bool.self, forKey: .isBot)

        let derivedIsChannel = rawIsChannel ?? (rawKind?.lowercased() == "channel" || decodedChatType == 2)
        let derivedIsGroup = rawIsGroup ?? (rawKind?.lowercased() == "group" || rawKind?.lowercased() == "supergroup" || decodedChatType == 1)

        self.isChannel = derivedIsChannel
        self.isGroup = derivedIsGroup && !derivedIsChannel
        self.isBot = Self.resolvedIsBot(
            rawIsBot: rawIsBot,
            isGroup: self.isGroup,
            isChannel: self.isChannel,
            kind: rawKind
        )

        if let rawKind, !rawKind.isEmpty {
            self.kind = rawKind
        } else if self.isChannel {
            self.kind = "channel"
        } else if self.isGroup {
            self.kind = "group"
        } else if self.isBot {
            self.kind = "bot"
        } else {
            self.kind = "private"
        }

        self.creatorId = try container.decodeIfPresent(UInt64.self, forKey: .creatorId) ?? 0
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.about = try container.decodeIfPresent(String.self, forKey: .about)
        self.isForum = try container.decodeIfPresent(Bool.self, forKey: .isForum) ?? false
        self.isArchived = try container.decodeIfPresent(Bool.self, forKey: .isArchived) ?? false
        self.isPinned = try container.decodeIfPresent(Bool.self, forKey: .isPinned) ?? false
        self.isMuted = try container.decodeIfPresent(Bool.self, forKey: .isMuted) ?? false
        self.muteUntilMs = try container.decodeIfPresent(Int64.self, forKey: .muteUntilMs)
        self.pts = try container.decodeIfPresent(UInt64.self, forKey: .pts) ?? 0
        self.lastMessageId = try container.decodeIfPresent(Int64.self, forKey: .lastMessageId) ?? 0

        if let preview = try container.decodeIfPresent(String.self, forKey: .lastMessagePreview) {
            self.lastMessagePreview = preview
        } else {
            self.lastMessagePreview = try container.decodeIfPresent(String.self, forKey: .lastMessage)
        }

        if let ms = try container.decodeIfPresent(Int64.self, forKey: .lastMessageDateMs), ms > 0 {
            self.lastMessageDateMs = FabushiWireDateParser.normalizeEpochToMs(ms)
        } else if let updatedRaw = try container.decodeIfPresent(FabushiJSONValue.self, forKey: .updatedAt),
                  let ms = FabushiWireDateParser.parseTimestampMs(from: updatedRaw) {
            self.lastMessageDateMs = ms
        } else {
            self.lastMessageDateMs = 0
        }

        self.unreadCount = try container.decodeIfPresent(Int.self, forKey: .unreadCount) ?? 0
        self.readInboxMaxId = try container.decodeIfPresent(Int64.self, forKey: .readInboxMaxId) ?? 0
        self.readOutboxMaxId = try container.decodeIfPresent(Int64.self, forKey: .readOutboxMaxId) ?? 0

        if let mc = try container.decodeIfPresent(Int.self, forKey: .memberCount) {
            self.memberCount = mc
        } else if let msc = try container.decodeIfPresent(Int.self, forKey: .membersCount) {
            self.memberCount = msc
        } else {
            self.memberCount = 0
        }

        self.pinnedMessageId = try container.decodeIfPresent(Int64.self, forKey: .pinnedMessageId)
        self.avatarColor = try container.decodeIfPresent(UInt32.self, forKey: .avatarColor)
        self.ttlSeconds = try container.decodeIfPresent(UInt32.self, forKey: .ttlSeconds) ?? 0
        self.draft = try container.decodeIfPresent(FabushiDraftRecord.self, forKey: .draft)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(chatId, forKey: .chatId)
        try container.encode(title, forKey: .title)
        try container.encode(kind, forKey: .kind)
        try container.encode(chatType, forKey: .chatType)
        try container.encode(creatorId, forKey: .creatorId)
        try container.encodeIfPresent(username, forKey: .username)
        try container.encodeIfPresent(about, forKey: .about)
        try container.encode(isGroup, forKey: .isGroup)
        try container.encode(isChannel, forKey: .isChannel)
        try container.encode(isBot, forKey: .isBot)
        try container.encode(isForum, forKey: .isForum)
        try container.encode(isArchived, forKey: .isArchived)
        try container.encode(isPinned, forKey: .isPinned)
        try container.encode(isMuted, forKey: .isMuted)
        try container.encodeIfPresent(muteUntilMs, forKey: .muteUntilMs)
        try container.encode(pts, forKey: .pts)
        try container.encode(lastMessageId, forKey: .lastMessageId)
        try container.encodeIfPresent(lastMessagePreview, forKey: .lastMessagePreview)
        try container.encode(lastMessageDateMs, forKey: .lastMessageDateMs)
        try container.encode(unreadCount, forKey: .unreadCount)
        try container.encode(readInboxMaxId, forKey: .readInboxMaxId)
        try container.encode(readOutboxMaxId, forKey: .readOutboxMaxId)
        try container.encode(memberCount, forKey: .memberCount)
        try container.encodeIfPresent(pinnedMessageId, forKey: .pinnedMessageId)
        try container.encodeIfPresent(avatarColor, forKey: .avatarColor)
        try container.encode(ttlSeconds, forKey: .ttlSeconds)
        try container.encodeIfPresent(draft, forKey: .draft)
    }

    /// Merges sparse online `ChatSummary` updates into richer cached local state without wiping
    /// local pinned/muted/archived/ttl/draft flags.
    public func mergedPreservingRichLocalFlags(from newer: FabushiMessagingChat) -> FabushiMessagingChat {
        var merged = newer
        if !newer.isPinned && self.isPinned && newer.lastMessageId == 0 {
            merged.isPinned = self.isPinned
        }
        if !newer.isArchived && self.isArchived && newer.lastMessageId == 0 {
            merged.isArchived = self.isArchived
        }
        if !newer.isMuted && self.isMuted && newer.lastMessageId == 0 {
            merged.isMuted = self.isMuted
            merged.muteUntilMs = self.muteUntilMs
        }
        if merged.about == nil { merged.about = self.about }
        if merged.username == nil { merged.username = self.username }
        if merged.memberCount == 0 { merged.memberCount = self.memberCount }
        if merged.ttlSeconds == 0 { merged.ttlSeconds = self.ttlSeconds }
        if merged.draft == nil { merged.draft = self.draft }
        if merged.pinnedMessageId == nil { merged.pinnedMessageId = self.pinnedMessageId }
        if merged.pts == 0 { merged.pts = self.pts }
        if merged.lastMessageId == 0 { merged.lastMessageId = self.lastMessageId }
        if merged.lastMessagePreview == nil { merged.lastMessagePreview = self.lastMessagePreview }
        if merged.lastMessageDateMs == 0 { merged.lastMessageDateMs = self.lastMessageDateMs }
        if merged.isGroup || merged.isChannel {
            merged.isBot = false
        }
        return merged
    }
}

// MARK: - Message Entity, Inline Keyboard, Cards & Content

public enum FabushiMessageEntityKind: Codable, Sendable, Equatable, Hashable {
    case bold
    case italic
    case underline
    case strikethrough
    case spoiler
    case code
    case pre(language: String?)
    case url
    case textUrl(url: String)
    case mention
    case hashtag
    case botCommand
    case customEmoji(documentId: UInt64)

    private enum CodingKeys: String, CodingKey {
        case type
        case value
    }

    private struct PreValue: Codable {
        let language: String?
    }

    private struct TextUrlValue: Codable {
        let url: String
    }

    private struct CustomEmojiValue: Codable {
        let documentId: UInt64
        enum CodingKeys: String, CodingKey {
            case documentId = "document_id"
        }
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(String.self, forKey: .type)
        switch type {
        case "Bold": self = .bold
        case "Italic": self = .italic
        case "Underline": self = .underline
        case "Strikethrough": self = .strikethrough
        case "Spoiler": self = .spoiler
        case "Code": self = .code
        case "Pre":
            let val = try container.decodeIfPresent(PreValue.self, forKey: .value)
            self = .pre(language: val?.language)
        case "Url": self = .url
        case "TextUrl":
            let val = try container.decode(TextUrlValue.self, forKey: .value)
            self = .textUrl(url: val.url)
        case "Mention": self = .mention
        case "Hashtag": self = .hashtag
        case "BotCommand": self = .botCommand
        case "CustomEmoji":
            let val = try container.decode(CustomEmojiValue.self, forKey: .value)
            self = .customEmoji(documentId: val.documentId)
        default:
            self = .code
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .bold:
            try container.encode("Bold", forKey: .type)
        case .italic:
            try container.encode("Italic", forKey: .type)
        case .underline:
            try container.encode("Underline", forKey: .type)
        case .strikethrough:
            try container.encode("Strikethrough", forKey: .type)
        case .spoiler:
            try container.encode("Spoiler", forKey: .type)
        case .code:
            try container.encode("Code", forKey: .type)
        case .pre(let language):
            try container.encode("Pre", forKey: .type)
            try container.encode(PreValue(language: language), forKey: .value)
        case .url:
            try container.encode("Url", forKey: .type)
        case .textUrl(let url):
            try container.encode("TextUrl", forKey: .type)
            try container.encode(TextUrlValue(url: url), forKey: .value)
        case .mention:
            try container.encode("Mention", forKey: .type)
        case .hashtag:
            try container.encode("Hashtag", forKey: .type)
        case .botCommand:
            try container.encode("BotCommand", forKey: .type)
        case .customEmoji(let documentId):
            try container.encode("CustomEmoji", forKey: .type)
            try container.encode(CustomEmojiValue(documentId: documentId), forKey: .value)
        }
    }
}

public struct FabushiMessageEntity: Codable, Sendable, Equatable, Hashable {
    public let offset: UInt32
    public let length: UInt32
    public let entityType: FabushiMessageEntityKind

    public enum CodingKeys: String, CodingKey {
        case offset
        case length
        case entityType = "entity_type"
    }

    public init(offset: UInt32, length: UInt32, entityType: FabushiMessageEntityKind) {
        self.offset = offset
        self.length = length
        self.entityType = entityType
    }
}

public struct FabushiInlineKeyboardButton: Codable, Sendable, Equatable, Hashable, Identifiable {
    public var id: String { "\(text)|\(callbackData ?? "")|\(url ?? "")|\(webAppUrl ?? "")" }

    public let text: String
    public let callbackData: String?
    public let url: String?
    public let webAppUrl: String?

    public enum CodingKeys: String, CodingKey {
        case text
        case callbackData = "callback_data"
        case url
        case webAppUrl = "web_app_url"
    }

    public init(text: String, callbackData: String? = nil, url: String? = nil, webAppUrl: String? = nil) {
        self.text = text
        self.callbackData = callbackData
        self.url = url
        self.webAppUrl = webAppUrl
    }
}

public struct FabushiMessageReaction: Codable, Sendable, Equatable, Hashable, Identifiable {
    public var id: String { emoji }

    public let emoji: String
    public var count: Int
    public var isChosenByMe: Bool

    private enum CodingKeys: String, CodingKey {
        case emoji
        case reaction
        case count
        case isChosenByMe = "is_chosen_by_me"
        case chosenByMe = "chosen_by_me"
        case chosen
        case me
    }

    public init(emoji: String, count: Int, isChosenByMe: Bool) {
        self.emoji = emoji
        self.count = count
        self.isChosenByMe = isChosenByMe
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.emoji = try container.decodeIfPresent(String.self, forKey: .emoji)
            ?? container.decodeIfPresent(String.self, forKey: .reaction)
            ?? ""
        self.count = try container.decodeIfPresent(Int.self, forKey: .count) ?? 1
        self.isChosenByMe = try container.decodeIfPresent(Bool.self, forKey: .isChosenByMe)
            ?? container.decodeIfPresent(Bool.self, forKey: .chosenByMe)
            ?? container.decodeIfPresent(Bool.self, forKey: .chosen)
            ?? container.decodeIfPresent(Bool.self, forKey: .me)
            ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(emoji, forKey: .emoji)
        try container.encode(count, forKey: .count)
        try container.encode(isChosenByMe, forKey: .isChosenByMe)
    }
}

public struct FabushiReplyHeader: Codable, Sendable, Equatable, Hashable {
    public let replyToMsgId: Int64
    public let quoteText: String?
    public let senderId: UInt64?
    public let senderName: String?

    private enum CodingKeys: String, CodingKey {
        case replyToMsgId = "reply_to_msg_id"
        case messageId = "message_id"
        case quoteText = "quote_text"
        case snippet
        case text
        case senderId = "sender_id"
        case senderName = "sender_name"
    }

    public init(replyToMsgId: Int64, quoteText: String? = nil, senderId: UInt64? = nil, senderName: String? = nil) {
        self.replyToMsgId = replyToMsgId
        self.quoteText = quoteText
        self.senderId = senderId
        self.senderName = senderName
    }

    public init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer(),
           let idOnly = try? single.decode(Int64.self) {
            self.replyToMsgId = idOnly
            self.quoteText = nil
            self.senderId = nil
            self.senderName = nil
            return
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.replyToMsgId = try container.decodeIfPresent(Int64.self, forKey: .replyToMsgId)
            ?? container.decodeIfPresent(Int64.self, forKey: .messageId)
            ?? 0
        self.quoteText = try container.decodeIfPresent(String.self, forKey: .quoteText)
            ?? container.decodeIfPresent(String.self, forKey: .snippet)
            ?? container.decodeIfPresent(String.self, forKey: .text)
        self.senderId = try container.decodeIfPresent(UInt64.self, forKey: .senderId)
        self.senderName = try container.decodeIfPresent(String.self, forKey: .senderName)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(replyToMsgId, forKey: .replyToMsgId)
        try container.encodeIfPresent(quoteText, forKey: .quoteText)
        try container.encodeIfPresent(senderId, forKey: .senderId)
        try container.encodeIfPresent(senderName, forKey: .senderName)
    }
}

public struct FabushiReplyTarget: Codable, Sendable, Equatable, Hashable {
    public let messageId: Int64
    public let senderName: String
    public let quoteText: String

    public init(messageId: Int64, senderName: String, quoteText: String) {
        self.messageId = messageId
        self.senderName = senderName
        self.quoteText = quoteText
    }
}

public struct FabushiForwardHeader: Codable, Sendable, Equatable, Hashable {
    public let fromChatId: Int64?
    public let fromMessageId: Int64?
    public let fromName: String?
    public let dateMs: Int64?

    private enum CodingKeys: String, CodingKey {
        case fromChatId = "from_chat_id"
        case fromMessageId = "from_message_id"
        case fromMsgId = "from_msg_id"
        case fromName = "from_name"
        case dateMs = "date_ms"
    }

    public init(fromChatId: Int64? = nil, fromMessageId: Int64? = nil, fromName: String? = nil, dateMs: Int64? = nil) {
        self.fromChatId = fromChatId
        self.fromMessageId = fromMessageId
        self.fromName = fromName
        self.dateMs = dateMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.fromChatId = try container.decodeIfPresent(Int64.self, forKey: .fromChatId)
        self.fromMessageId = try container.decodeIfPresent(Int64.self, forKey: .fromMessageId)
            ?? container.decodeIfPresent(Int64.self, forKey: .fromMsgId)
        self.fromName = try container.decodeIfPresent(String.self, forKey: .fromName)
        self.dateMs = try container.decodeIfPresent(Int64.self, forKey: .dateMs)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encodeIfPresent(fromChatId, forKey: .fromChatId)
        try container.encodeIfPresent(fromMessageId, forKey: .fromMessageId)
        try container.encodeIfPresent(fromName, forKey: .fromName)
        try container.encodeIfPresent(dateMs, forKey: .dateMs)
    }
}

public struct FabushiMediaThumbnail: Codable, Sendable, Equatable, Hashable {
    public let fileId: String
    public let width: UInt32
    public let height: UInt32
    public let dataBase64: String?

    public enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case width
        case height
        case dataBase64 = "data_base64"
    }

    public init(fileId: String, width: UInt32, height: UInt32, dataBase64: String? = nil) {
        self.fileId = fileId
        self.width = width
        self.height = height
        self.dataBase64 = dataBase64
    }
}

public struct FabushiMediaItemPayload: Codable, Sendable, Equatable, Hashable {
    public let fileId: String
    public let fileName: String?
    public let mimeType: String
    public let sizeBytes: UInt64
    public let width: UInt32?
    public let height: UInt32?
    public let durationSeconds: UInt32?
    public let thumbnail: FabushiMediaThumbnail?
    public let caption: String?

    public enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case fileName = "file_name"
        case mimeType = "mime_type"
        case sizeBytes = "size_bytes"
        case width
        case height
        case durationSeconds = "duration_seconds"
        case thumbnail
        case caption
    }

    public init(
        fileId: String,
        fileName: String? = nil,
        mimeType: String,
        sizeBytes: UInt64,
        width: UInt32? = nil,
        height: UInt32? = nil,
        durationSeconds: UInt32? = nil,
        thumbnail: FabushiMediaThumbnail? = nil,
        caption: String? = nil
    ) {
        self.fileId = fileId
        self.fileName = fileName
        self.mimeType = mimeType
        self.sizeBytes = sizeBytes
        self.width = width
        self.height = height
        self.durationSeconds = durationSeconds
        self.thumbnail = thumbnail
        self.caption = caption
    }
}

public struct FabushiVoiceItemPayload: Codable, Sendable, Equatable, Hashable {
    public let fileId: String
    public let durationSeconds: UInt32
    public let waveform: [UInt8]
    public let mimeType: String
    public let sizeBytes: UInt64

    public enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case durationSeconds = "duration_seconds"
        case waveform
        case mimeType = "mime_type"
        case sizeBytes = "size_bytes"
    }

    public init(fileId: String, durationSeconds: UInt32, waveform: [UInt8], mimeType: String, sizeBytes: UInt64) {
        self.fileId = fileId
        self.durationSeconds = durationSeconds
        self.waveform = waveform
        self.mimeType = mimeType
        self.sizeBytes = sizeBytes
    }
}

public struct FabushiVideoNotePayload: Codable, Sendable, Equatable, Hashable {
    public let fileId: String
    public let durationSeconds: UInt32
    public let diameter: UInt32
    public let thumbnail: FabushiMediaThumbnail?
    public let sizeBytes: UInt64

    public enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case durationSeconds = "duration_seconds"
        case diameter
        case thumbnail
        case sizeBytes = "size_bytes"
    }

    public init(fileId: String, durationSeconds: UInt32, diameter: UInt32, thumbnail: FabushiMediaThumbnail? = nil, sizeBytes: UInt64) {
        self.fileId = fileId
        self.durationSeconds = durationSeconds
        self.diameter = diameter
        self.thumbnail = thumbnail
        self.sizeBytes = sizeBytes
    }
}

public struct FabushiPollOption: Codable, Sendable, Equatable, Hashable, Identifiable {
    public var id: UInt8 { optionId }

    public let optionId: UInt8
    public let text: String
    public let voteCount: Int
    public let isChosen: Bool
    public let percentage: Double

    public enum CodingKeys: String, CodingKey {
        case optionId = "option_id"
        case text
        case voteCount = "vote_count"
        case isChosen = "is_chosen"
        case percentage
    }

    public init(optionId: UInt8, text: String, voteCount: Int = 0, isChosen: Bool = false, percentage: Double = 0) {
        self.optionId = optionId
        self.text = text
        self.voteCount = voteCount
        self.isChosen = isChosen
        self.percentage = percentage
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.optionId = try container.decodeIfPresent(UInt8.self, forKey: .optionId) ?? 0
        self.text = try container.decodeIfPresent(String.self, forKey: .text) ?? ""
        self.voteCount = try container.decodeIfPresent(Int.self, forKey: .voteCount) ?? 0
        self.isChosen = try container.decodeIfPresent(Bool.self, forKey: .isChosen) ?? false
        self.percentage = try container.decodeIfPresent(Double.self, forKey: .percentage) ?? 0
    }
}

public struct FabushiPollCard: Codable, Sendable, Equatable, Hashable {
    public let pollId: UInt64
    public let question: String
    public let options: [FabushiPollOption]
    public let totalVoters: Int
    public let isClosed: Bool
    public let isAnonymous: Bool
    public let allowsMultipleAnswers: Bool
    public let isQuiz: Bool
    public let correctOptionId: UInt8?
    public let solution: String?

    public enum CodingKeys: String, CodingKey {
        case pollId = "poll_id"
        case id
        case question
        case options
        case totalVoters = "total_voters"
        case isClosed = "is_closed"
        case isAnonymous = "is_anonymous"
        case allowsMultipleAnswers = "allows_multiple_answers"
        case isQuiz = "is_quiz"
        case correctOptionId = "correct_option_id"
        case solution
    }

    public init(
        pollId: UInt64,
        question: String,
        options: [FabushiPollOption],
        totalVoters: Int = 0,
        isClosed: Bool = false,
        isAnonymous: Bool = true,
        allowsMultipleAnswers: Bool = false,
        isQuiz: Bool = false,
        correctOptionId: UInt8? = nil,
        solution: String? = nil
    ) {
        self.pollId = pollId
        self.question = question
        self.options = options
        self.totalVoters = totalVoters
        self.isClosed = isClosed
        self.isAnonymous = isAnonymous
        self.allowsMultipleAnswers = allowsMultipleAnswers
        self.isQuiz = isQuiz
        self.correctOptionId = correctOptionId
        self.solution = solution
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.pollId = try container.decodeIfPresent(UInt64.self, forKey: .pollId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .id)
            ?? 0
        self.question = try container.decodeIfPresent(String.self, forKey: .question) ?? ""
        self.options = try container.decodeIfPresent([FabushiPollOption].self, forKey: .options) ?? []
        self.totalVoters = try container.decodeIfPresent(Int.self, forKey: .totalVoters) ?? 0
        self.isClosed = try container.decodeIfPresent(Bool.self, forKey: .isClosed) ?? false
        self.isAnonymous = try container.decodeIfPresent(Bool.self, forKey: .isAnonymous) ?? true
        self.allowsMultipleAnswers = try container.decodeIfPresent(Bool.self, forKey: .allowsMultipleAnswers) ?? false
        self.isQuiz = try container.decodeIfPresent(Bool.self, forKey: .isQuiz) ?? false
        self.correctOptionId = try container.decodeIfPresent(UInt8.self, forKey: .correctOptionId)
        self.solution = try container.decodeIfPresent(String.self, forKey: .solution)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(pollId, forKey: .pollId)
        try container.encode(question, forKey: .question)
        try container.encode(options, forKey: .options)
        try container.encode(totalVoters, forKey: .totalVoters)
        try container.encode(isClosed, forKey: .isClosed)
        try container.encode(isAnonymous, forKey: .isAnonymous)
        try container.encode(allowsMultipleAnswers, forKey: .allowsMultipleAnswers)
        try container.encode(isQuiz, forKey: .isQuiz)
        try container.encodeIfPresent(correctOptionId, forKey: .correctOptionId)
        try container.encodeIfPresent(solution, forKey: .solution)
    }
}

public struct FabushiTodoItem: Codable, Sendable, Equatable, Hashable, Identifiable {
    public let id: String
    public let title: String
    public var completed: Bool
    public let assignee: String?
    public let completedAtMs: Int64?

    public enum CodingKeys: String, CodingKey {
        case id
        case title
        case completed
        case assignee
        case completedAtMs = "completed_at_ms"
    }

    public init(id: String, title: String, completed: Bool, assignee: String? = nil, completedAtMs: Int64? = nil) {
        self.id = id
        self.title = title
        self.completed = completed
        self.assignee = assignee
        self.completedAtMs = completedAtMs
    }
}

public struct FabushiTodoActionButton: Codable, Sendable, Equatable, Hashable, Identifiable {
    public let id: String
    public let label: String
    public let actionType: String
    public let payload: String

    public enum CodingKeys: String, CodingKey {
        case id
        case label
        case actionType = "action_type"
        case payload
    }

    public init(id: String, label: String, actionType: String, payload: String) {
        self.id = id
        self.label = label
        self.actionType = actionType
        self.payload = payload
    }
}

public struct FabushiTodoCard: Codable, Sendable, Equatable, Hashable {
    public let id: String
    public let title: String
    public var items: [FabushiTodoItem]
    public let completedCount: Int
    public let actionButtons: [FabushiTodoActionButton]

    public enum CodingKeys: String, CodingKey {
        case id
        case listId = "list_id"
        case title
        case items
        case completedCount = "completed_count"
        case actionButtons = "action_buttons"
    }

    public init(
        id: String,
        title: String,
        items: [FabushiTodoItem],
        completedCount: Int = 0,
        actionButtons: [FabushiTodoActionButton] = []
    ) {
        self.id = id
        self.title = title
        self.items = items
        self.completedCount = completedCount
        self.actionButtons = actionButtons
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try container.decodeIfPresent(String.self, forKey: .id)
            ?? container.decodeIfPresent(String.self, forKey: .listId)
            ?? ""
        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        self.items = try container.decodeIfPresent([FabushiTodoItem].self, forKey: .items) ?? []
        self.completedCount = try container.decodeIfPresent(Int.self, forKey: .completedCount)
            ?? self.items.filter(\.completed).count
        self.actionButtons = try container.decodeIfPresent([FabushiTodoActionButton].self, forKey: .actionButtons) ?? []
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(title, forKey: .title)
        try container.encode(items, forKey: .items)
        try container.encode(completedCount, forKey: .completedCount)
        try container.encode(actionButtons, forKey: .actionButtons)
    }
}

public struct FabushiInvoicePrice: Codable, Sendable, Equatable, Hashable {
    public let label: String
    public let amountCents: Int64

    public enum CodingKeys: String, CodingKey {
        case label
        case amountCents = "amount_cents"
    }

    public init(label: String, amountCents: Int64) {
        self.label = label
        self.amountCents = amountCents
    }
}

public struct FabushiInvoiceCard: Codable, Sendable, Equatable, Hashable {
    public let invoiceId: String
    public let title: String
    public let descriptionText: String
    public let currency: String
    public let amountCents: Int64
    public let status: String
    public let isPaid: Bool
    public let prices: [FabushiInvoicePrice]

    public enum CodingKeys: String, CodingKey {
        case invoiceId = "invoice_id"
        case id
        case title
        case descriptionText = "description"
        case currency
        case amountCents = "amount_cents"
        case totalAmount = "total_amount"
        case status
        case isPaid = "is_paid"
        case prices
    }

    public init(
        invoiceId: String,
        title: String,
        descriptionText: String,
        currency: String,
        amountCents: Int64,
        status: String = "pending",
        isPaid: Bool = false,
        prices: [FabushiInvoicePrice] = []
    ) {
        self.invoiceId = invoiceId
        self.title = title
        self.descriptionText = descriptionText
        self.currency = currency
        self.amountCents = amountCents
        self.status = status
        self.isPaid = isPaid
        self.prices = prices
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.invoiceId = try container.decodeIfPresent(String.self, forKey: .invoiceId)
            ?? container.decodeIfPresent(String.self, forKey: .id)
            ?? ""
        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        self.descriptionText = try container.decodeIfPresent(String.self, forKey: .descriptionText) ?? ""
        self.currency = try container.decodeIfPresent(String.self, forKey: .currency) ?? "USD"
        self.amountCents = try container.decodeIfPresent(Int64.self, forKey: .amountCents)
            ?? container.decodeIfPresent(Int64.self, forKey: .totalAmount)
            ?? 0
        self.status = try container.decodeIfPresent(String.self, forKey: .status) ?? "pending"
        self.isPaid = try container.decodeIfPresent(Bool.self, forKey: .isPaid) ?? false
        self.prices = try container.decodeIfPresent([FabushiInvoicePrice].self, forKey: .prices) ?? []
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(invoiceId, forKey: .invoiceId)
        try container.encode(title, forKey: .title)
        try container.encode(descriptionText, forKey: .descriptionText)
        try container.encode(currency, forKey: .currency)
        try container.encode(amountCents, forKey: .amountCents)
        try container.encode(status, forKey: .status)
        try container.encode(isPaid, forKey: .isPaid)
        try container.encode(prices, forKey: .prices)
    }
}

public struct FabushiGenerativeCard: Codable, Sendable, Equatable, Hashable {
    public let cardId: String
    public let title: String
    public let subtitle: String?
    public let layout: String
    public let status: String?
    public let componentsJSON: String

    private enum CodingKeys: String, CodingKey {
        case cardId = "card_id"
        case title
        case subtitle
        case layout
        case status
        case components
    }

    public init(
        cardId: String,
        title: String,
        subtitle: String? = nil,
        layout: String = "VerticalStack",
        status: String? = nil,
        componentsJSON: String = "[]"
    ) {
        self.cardId = cardId
        self.title = title
        self.subtitle = subtitle
        self.layout = layout
        self.status = status
        self.componentsJSON = componentsJSON
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.cardId = try container.decodeIfPresent(String.self, forKey: .cardId) ?? ""
        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        self.subtitle = try container.decodeIfPresent(String.self, forKey: .subtitle)
        self.layout = try container.decodeIfPresent(String.self, forKey: .layout) ?? "VerticalStack"
        self.status = try container.decodeIfPresent(String.self, forKey: .status)
        if let raw = try container.decodeIfPresent(FabushiJSONValue.self, forKey: .components) {
            self.componentsJSON = raw.toJSONString()
        } else {
            self.componentsJSON = "[]"
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(cardId, forKey: .cardId)
        try container.encode(title, forKey: .title)
        try container.encodeIfPresent(subtitle, forKey: .subtitle)
        try container.encode(layout, forKey: .layout)
        try container.encodeIfPresent(status, forKey: .status)
        if let data = componentsJSON.data(using: .utf8),
           let parsed = try? JSONDecoder().decode(FabushiJSONValue.self, from: data) {
            try container.encode(parsed, forKey: .components)
        } else {
            try container.encode([FabushiJSONValue](), forKey: .components)
        }
    }
}

public enum FabushiMessageContent: Codable, Sendable, Equatable, Hashable {
    case text(text: String, entities: [FabushiMessageEntity])
    case media(FabushiMediaItemPayload)
    case voice(FabushiVoiceItemPayload)
    case videoNote(FabushiVideoNotePayload)
    case todo(FabushiTodoCard)
    case poll(FabushiPollCard)
    case invoice(FabushiInvoiceCard)
    case generativeCard(FabushiGenerativeCard)
    case photo(fileId: String, caption: String?, width: UInt32, height: UInt32)
    case video(fileId: String, duration: UInt32, width: UInt32, height: UInt32, caption: String?)
    case audio(fileId: String, duration: UInt32, title: String?, performer: String?)
    case document(fileId: String, fileName: String, mimeType: String, sizeBytes: UInt64)
    case encrypted(rawEnvelopeJSON: String)
    case extended(kind: String, rawJSON: String)

    private enum CodingKeys: String, CodingKey {
        case type
        case data
    }

    private struct TextData: Codable {
        let text: String
        let entities: [FabushiMessageEntity]?
    }

    private struct PhotoData: Codable {
        let fileId: String
        let caption: String?
        let width: UInt32
        let height: UInt32
        enum CodingKeys: String, CodingKey {
            case fileId = "file_id"
            case caption
            case width
            case height
        }
    }

    private struct VideoData: Codable {
        let fileId: String
        let duration: UInt32
        let width: UInt32
        let height: UInt32
        let caption: String?
        enum CodingKeys: String, CodingKey {
            case fileId = "file_id"
            case duration
            case width
            case height
            case caption
        }
    }

    private struct AudioData: Codable {
        let fileId: String
        let duration: UInt32
        let title: String?
        let performer: String?
        enum CodingKeys: String, CodingKey {
            case fileId = "file_id"
            case duration
            case title
            case performer
        }
    }

    private struct DocumentData: Codable {
        let fileId: String
        let fileName: String
        let mimeType: String
        let sizeBytes: UInt64
        enum CodingKeys: String, CodingKey {
            case fileId = "file_id"
            case fileName = "file_name"
            case mimeType = "mime_type"
            case sizeBytes = "size_bytes"
        }
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let type = try container.decodeIfPresent(String.self, forKey: .type) {
            switch type {
            case "Text":
                let payload = try container.decode(TextData.self, forKey: .data)
                self = .text(text: payload.text, entities: payload.entities ?? [])
                return
            case "Media":
                self = .media(try container.decode(FabushiMediaItemPayload.self, forKey: .data))
                return
            case "Voice":
                self = .voice(try container.decode(FabushiVoiceItemPayload.self, forKey: .data))
                return
            case "VideoNote":
                self = .videoNote(try container.decode(FabushiVideoNotePayload.self, forKey: .data))
                return
            case "Todo":
                self = .todo(try container.decode(FabushiTodoCard.self, forKey: .data))
                return
            case "Poll":
                self = .poll(try container.decode(FabushiPollCard.self, forKey: .data))
                return
            case "Invoice":
                self = .invoice(try container.decode(FabushiInvoiceCard.self, forKey: .data))
                return
            case "GenerativeCard":
                self = .generativeCard(try container.decode(FabushiGenerativeCard.self, forKey: .data))
                return
            case "Photo":
                let p = try container.decode(PhotoData.self, forKey: .data)
                self = .photo(fileId: p.fileId, caption: p.caption, width: p.width, height: p.height)
                return
            case "Video":
                let v = try container.decode(VideoData.self, forKey: .data)
                self = .video(fileId: v.fileId, duration: v.duration, width: v.width, height: v.height, caption: v.caption)
                return
            case "Audio":
                let a = try container.decode(AudioData.self, forKey: .data)
                self = .audio(fileId: a.fileId, duration: a.duration, title: a.title, performer: a.performer)
                return
            case "Document":
                let d = try container.decode(DocumentData.self, forKey: .data)
                self = .document(fileId: d.fileId, fileName: d.fileName, mimeType: d.mimeType, sizeBytes: d.sizeBytes)
                return
            case "Encrypted":
                let raw = (try? container.decode(FabushiJSONValue.self, forKey: .data)) ?? .null
                self = .encrypted(rawEnvelopeJSON: raw.toJSONString())
                return
            default:
                let raw = (try? FabushiJSONValue(from: decoder)) ?? .null
                self = .extended(kind: type, rawJSON: raw.toJSONString())
                return
            }
        }
        let raw = (try? FabushiJSONValue(from: decoder)) ?? .null
        let actionType = raw["action_type"]?.stringValue ?? raw["kind"]?.stringValue ?? "extended"
        self = .extended(kind: actionType, rawJSON: raw.toJSONString())
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .text(let text, let entities):
            try container.encode("Text", forKey: .type)
            try container.encode(TextData(text: text, entities: entities), forKey: .data)
        case .media(let item):
            try container.encode("Media", forKey: .type)
            try container.encode(item, forKey: .data)
        case .voice(let item):
            try container.encode("Voice", forKey: .type)
            try container.encode(item, forKey: .data)
        case .videoNote(let item):
            try container.encode("VideoNote", forKey: .type)
            try container.encode(item, forKey: .data)
        case .todo(let card):
            try container.encode("Todo", forKey: .type)
            try container.encode(card, forKey: .data)
        case .poll(let card):
            try container.encode("Poll", forKey: .type)
            try container.encode(card, forKey: .data)
        case .invoice(let card):
            try container.encode("Invoice", forKey: .type)
            try container.encode(card, forKey: .data)
        case .generativeCard(let card):
            try container.encode("GenerativeCard", forKey: .type)
            try container.encode(card, forKey: .data)
        case .photo(let fileId, let caption, let width, let height):
            try container.encode("Photo", forKey: .type)
            try container.encode(PhotoData(fileId: fileId, caption: caption, width: width, height: height), forKey: .data)
        case .video(let fileId, let duration, let width, let height, let caption):
            try container.encode("Video", forKey: .type)
            try container.encode(VideoData(fileId: fileId, duration: duration, width: width, height: height, caption: caption), forKey: .data)
        case .audio(let fileId, let duration, let title, let performer):
            try container.encode("Audio", forKey: .type)
            try container.encode(AudioData(fileId: fileId, duration: duration, title: title, performer: performer), forKey: .data)
        case .document(let fileId, let fileName, let mimeType, let sizeBytes):
            try container.encode("Document", forKey: .type)
            try container.encode(DocumentData(fileId: fileId, fileName: fileName, mimeType: mimeType, sizeBytes: sizeBytes), forKey: .data)
        case .encrypted(let rawEnvelopeJSON):
            try container.encode("Encrypted", forKey: .type)
            if let data = rawEnvelopeJSON.data(using: .utf8),
               let val = try? JSONDecoder().decode(FabushiJSONValue.self, from: data) {
                try container.encode(val, forKey: .data)
            } else {
                try container.encode(FabushiJSONValue.null, forKey: .data)
            }
        case .extended(let kind, let rawJSON):
            try container.encode(kind, forKey: .type)
            if let data = rawJSON.data(using: .utf8),
               let val = try? JSONDecoder().decode(FabushiJSONValue.self, from: data) {
                try container.encode(val, forKey: .data)
            } else {
                try container.encode(FabushiJSONValue.null, forKey: .data)
            }
        }
    }

    public var previewSummary: String {
        switch self {
        case .text(let text, _): return text
        case .photo(_, let caption, _, _): return caption ?? "[Photo]"
        case .video(_, _, _, _, let caption): return caption ?? "[Video]"
        case .audio(_, _, let title, _): return title ?? "[Audio]"
        case .document(_, let fileName, _, _): return fileName
        case .media(let m): return m.caption ?? m.fileName ?? "[Media]"
        case .voice: return "[Voice]"
        case .videoNote: return "[Video Message]"
        case .poll(let poll): return poll.question
        case .todo(let todo): return todo.title
        case .invoice(let invoice): return invoice.title
        case .generativeCard(let card): return card.title
        case .encrypted: return "[Encrypted]"
        case .extended(let kind, _): return "[\(kind)]"
        }
    }
}

// MARK: - Message Record (Dual Online `HistoricalMessage` + Offline/WS `MessageItem` + Local Echo)

public enum FabushiMessageDeliveryStatus: String, Codable, Sendable, Equatable, Hashable {
    case pending
    case sending
    case sent
    case read
    case failed

    public init(wireStatus: String?) {
        switch wireStatus?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "pending", "queued": self = .pending
        case "sending", "inflight", "in_flight": self = .sending
        case "read", "seen": self = .read
        case "failed", "error": self = .failed
        default: self = .sent
        }
    }
}

public struct FabushiMessageRecord: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: Int64 { messageId }

    public var messageId: Int64
    public var chatId: Int64
    public var senderId: UInt64
    public var topicId: Int64?
    public var text: String
    public var entities: [FabushiMessageEntity]
    public var content: FabushiMessageContent?
    public var dateMs: Int64
    public var editDateMs: Int64?
    public var isOutgoing: Bool
    public var replyTo: FabushiReplyHeader?
    public var forwardInfo: FabushiForwardHeader?
    public var reactions: [FabushiMessageReaction]
    public var inlineKeyboard: [[FabushiInlineKeyboardButton]]
    public var clientNonce: UInt64?
    public var deliveryStatus: FabushiMessageDeliveryStatus
    public var isDeletedTombstone: Bool
    public var ttlSeconds: UInt32?
    public var expiresAtMs: Int64?
    public var sendAsPeerId: UInt64?

    public init(
        messageId: Int64,
        chatId: Int64,
        senderId: UInt64,
        topicId: Int64? = nil,
        text: String,
        entities: [FabushiMessageEntity] = [],
        content: FabushiMessageContent? = nil,
        dateMs: Int64,
        editDateMs: Int64? = nil,
        isOutgoing: Bool = false,
        replyTo: FabushiReplyHeader? = nil,
        forwardInfo: FabushiForwardHeader? = nil,
        reactions: [FabushiMessageReaction] = [],
        inlineKeyboard: [[FabushiInlineKeyboardButton]] = [],
        clientNonce: UInt64? = nil,
        deliveryStatus: FabushiMessageDeliveryStatus = .sent,
        isDeletedTombstone: Bool = false,
        ttlSeconds: UInt32? = nil,
        expiresAtMs: Int64? = nil,
        sendAsPeerId: UInt64? = nil
    ) {
        self.messageId = messageId
        self.chatId = chatId
        self.senderId = senderId
        self.topicId = topicId
        self.text = text
        self.entities = entities
        self.content = content
        self.dateMs = dateMs
        self.editDateMs = editDateMs
        self.isOutgoing = isOutgoing
        self.replyTo = replyTo
        self.forwardInfo = forwardInfo
        self.reactions = reactions
        self.inlineKeyboard = inlineKeyboard
        self.clientNonce = clientNonce
        self.deliveryStatus = deliveryStatus
        self.isDeletedTombstone = isDeletedTombstone
        self.ttlSeconds = ttlSeconds
        self.expiresAtMs = expiresAtMs
        self.sendAsPeerId = sendAsPeerId
    }

    private enum CodingKeys: String, CodingKey {
        case messageId = "message_id"
        case id
        case chatId = "chat_id"
        case senderId = "sender_id"
        case topicId = "topic_id"
        case text
        case body
        case entities
        case content
        case date
        case dateMs = "date_ms"
        case timestamp
        case editDate = "edit_date"
        case editDateMs = "edit_date_ms"
        case editTimestamp = "edit_timestamp"
        case isOutgoing = "is_outgoing"
        case replyTo = "reply_to"
        case replyToInfo = "reply_to_info"
        case replyToMessageId = "reply_to_message_id"
        case forwardInfo = "forward_info"
        case reactions
        case inlineKeyboard = "inline_keyboard"
        case clientNonce = "client_nonce"
        case status
        case deliveryStatus = "delivery_status"
        case isDeletedTombstone = "is_deleted_tombstone"
        case ttlSeconds = "ttl_seconds"
        case expiresAtMs = "expires_at_ms"
        case sendAsPeerId = "send_as_peer_id"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)

        if let mid = try? container.decode(Int64.self, forKey: .messageId) {
            self.messageId = mid
        } else if let midU = try? container.decode(UInt64.self, forKey: .messageId) {
            self.messageId = Int64(bitPattern: midU)
        } else if let idVal = try? container.decode(Int64.self, forKey: .id) {
            self.messageId = idVal
        } else if let idU = try? container.decode(UInt64.self, forKey: .id) {
            self.messageId = Int64(bitPattern: idU)
        } else {
            self.messageId = 0
        }

        if let cid = try? container.decode(Int64.self, forKey: .chatId) {
            self.chatId = cid
        } else if let cidU = try? container.decode(UInt64.self, forKey: .chatId) {
            self.chatId = Int64(bitPattern: cidU)
        } else {
            self.chatId = 0
        }

        if let sid = try? container.decode(UInt64.self, forKey: .senderId) {
            self.senderId = sid
        } else if let sidI = try? container.decode(Int64.self, forKey: .senderId), sidI >= 0 {
            self.senderId = UInt64(sidI)
        } else {
            self.senderId = 0
        }

        self.topicId = try container.decodeIfPresent(Int64.self, forKey: .topicId)

        let decodedContent = try? container.decodeIfPresent(FabushiMessageContent.self, forKey: .content)
        self.content = decodedContent

        let primaryText = try container.decodeIfPresent(String.self, forKey: .text)
        let fallbackBody = try container.decodeIfPresent(String.self, forKey: .body)
        var resolvedText = primaryText ?? fallbackBody ?? ""
        var resolvedEntities = try container.decodeIfPresent([FabushiMessageEntity].self, forKey: .entities) ?? []

        if case .text(let nestedText, let nestedEntities)? = decodedContent {
            if resolvedText.isEmpty {
                resolvedText = nestedText
            }
            if resolvedEntities.isEmpty {
                resolvedEntities = nestedEntities
            }
        } else if resolvedText.isEmpty, let decodedContent {
            resolvedText = decodedContent.previewSummary
        }

        self.text = resolvedText
        self.entities = resolvedEntities

        let rawDate = (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .dateMs))
            ?? (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .date))
            ?? (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .timestamp))
        self.dateMs = FabushiWireDateParser.parseTimestampMs(from: rawDate) ?? 0

        let rawEditDate = (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .editDateMs))
            ?? (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .editDate))
            ?? (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .editTimestamp))
        self.editDateMs = FabushiWireDateParser.parseTimestampMs(from: rawEditDate)

        self.isOutgoing = try container.decodeIfPresent(Bool.self, forKey: .isOutgoing) ?? false

        if let info = try? container.decodeIfPresent(FabushiReplyHeader.self, forKey: .replyToInfo), info.replyToMsgId != 0 {
            self.replyTo = info
        } else if let header = try? container.decodeIfPresent(FabushiReplyHeader.self, forKey: .replyTo), header.replyToMsgId != 0 {
            self.replyTo = header
        } else if let replyId = try? container.decodeIfPresent(Int64.self, forKey: .replyToMessageId), replyId != 0 {
            self.replyTo = FabushiReplyHeader(replyToMsgId: replyId)
        } else {
            self.replyTo = nil
        }

        self.forwardInfo = try? container.decodeIfPresent(FabushiForwardHeader.self, forKey: .forwardInfo)
        self.reactions = (try? container.decodeIfPresent([FabushiMessageReaction].self, forKey: .reactions)) ?? []
        self.inlineKeyboard = (try? container.decodeIfPresent([[FabushiInlineKeyboardButton]].self, forKey: .inlineKeyboard)) ?? []
        self.clientNonce = try container.decodeIfPresent(UInt64.self, forKey: .clientNonce)

        if let ds = try? container.decodeIfPresent(FabushiMessageDeliveryStatus.self, forKey: .deliveryStatus) {
            self.deliveryStatus = ds
        } else {
            let statusStr = try container.decodeIfPresent(String.self, forKey: .status)
            self.deliveryStatus = FabushiMessageDeliveryStatus(wireStatus: statusStr)
        }

        self.isDeletedTombstone = try container.decodeIfPresent(Bool.self, forKey: .isDeletedTombstone) ?? false
        self.ttlSeconds = try container.decodeIfPresent(UInt32.self, forKey: .ttlSeconds)
        self.expiresAtMs = try container.decodeIfPresent(Int64.self, forKey: .expiresAtMs)
        self.sendAsPeerId = try container.decodeIfPresent(UInt64.self, forKey: .sendAsPeerId)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(messageId, forKey: .messageId)
        try container.encode(chatId, forKey: .chatId)
        try container.encode(senderId, forKey: .senderId)
        try container.encodeIfPresent(topicId, forKey: .topicId)
        try container.encode(text, forKey: .text)
        try container.encode(entities, forKey: .entities)
        try container.encodeIfPresent(content, forKey: .content)
        try container.encode(dateMs, forKey: .dateMs)
        try container.encodeIfPresent(editDateMs, forKey: .editDateMs)
        try container.encode(isOutgoing, forKey: .isOutgoing)
        try container.encodeIfPresent(replyTo, forKey: .replyTo)
        try container.encodeIfPresent(forwardInfo, forKey: .forwardInfo)
        try container.encode(reactions, forKey: .reactions)
        try container.encode(inlineKeyboard, forKey: .inlineKeyboard)
        try container.encodeIfPresent(clientNonce, forKey: .clientNonce)
        try container.encode(deliveryStatus, forKey: .deliveryStatus)
        try container.encode(isDeletedTombstone, forKey: .isDeletedTombstone)
        try container.encodeIfPresent(ttlSeconds, forKey: .ttlSeconds)
        try container.encodeIfPresent(expiresAtMs, forKey: .expiresAtMs)
        try container.encodeIfPresent(sendAsPeerId, forKey: .sendAsPeerId)
    }

    /// Dual-format event payload decoder (TRAP #2): decodes BOTH live WS `MessageItem`
    /// (`content: {type: Text, data: ...}`, RFC3339 `date`, `reply_to: {reply_to_msg_id, quote_text}`)
    /// AND flat `messages.send` local echo frames directly from `event.rawPayloadJSON`.
    public static func decodeFromDomainEventPayload(_ rawJSON: String, fallbackChatId: Int64 = 0) -> FabushiMessageRecord? {
        guard let data = rawJSON.data(using: .utf8) else { return nil }
        guard var decoded = try? JSONDecoder().decode(FabushiMessageRecord.self, from: data) else {
            return nil
        }
        if decoded.chatId == 0 && fallbackChatId != 0 {
            decoded.chatId = fallbackChatId
        }
        return decoded
    }

    /// Parses `reactionUpdated` payload directly from `event.rawPayloadJSON` preserving `is_chosen_by_me` (TRAP #2).
    public static func decodeReactionsFromEventPayload(_ rawJSON: String) -> (chatId: Int64, messageId: Int64, reactions: [FabushiMessageReaction])? {
        struct WireReactionEvent: Decodable {
            let chatId: Int64
            let messageId: Int64
            let reactions: [FabushiMessageReaction]?
            enum CodingKeys: String, CodingKey {
                case chatId = "chat_id"
                case messageId = "message_id"
                case reactions
            }
        }
        guard let data = rawJSON.data(using: .utf8),
              let wire = try? JSONDecoder().decode(WireReactionEvent.self, from: data)
        else {
            return nil
        }
        return (wire.chatId, wire.messageId, wire.reactions ?? [])
    }
}

// MARK: - Rich Send Request (`messages.sendDurable`, `D14-03` TRAP #1)

public struct FabushiRichSendMessageRequest: Codable, Sendable, Equatable {
    public let chatId: Int64
    public var text: String?
    public var content: FabushiMessageContent?
    public var entities: [FabushiMessageEntity]
    public var replyTo: Int64?
    public var quoteText: String?
    public var topicId: Int64?
    public var clientNonce: UInt64?
    public var forwardFromChatId: Int64?
    public var forwardFromMsgId: Int64?
    public var silent: Bool
    public var scheduledAtMs: Int64?
    public var inlineKeyboard: [[FabushiInlineKeyboardButton]]?
    public var sendAsPeerId: UInt64?
    public var skipServerBotReply: Bool

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case text
        case content
        case entities
        case replyTo = "reply_to"
        case quoteText = "quote_text"
        case topicId = "topic_id"
        case clientNonce = "client_nonce"
        case forwardFromChatId = "forward_from_chat_id"
        case forwardFromMsgId = "forward_from_msg_id"
        case silent
        case scheduledAtMs = "scheduled_at_ms"
        case inlineKeyboard = "inline_keyboard"
        case sendAsPeerId = "send_as_peer_id"
        case skipServerBotReply = "skip_server_bot_reply"
    }

    public init(
        chatId: Int64,
        text: String? = nil,
        content: FabushiMessageContent? = nil,
        entities: [FabushiMessageEntity] = [],
        replyTo: Int64? = nil,
        quoteText: String? = nil,
        topicId: Int64? = nil,
        clientNonce: UInt64? = nil,
        forwardFromChatId: Int64? = nil,
        forwardFromMsgId: Int64? = nil,
        silent: Bool = false,
        scheduledAtMs: Int64? = nil,
        inlineKeyboard: [[FabushiInlineKeyboardButton]]? = nil,
        sendAsPeerId: UInt64? = nil,
        skipServerBotReply: Bool = false
    ) {
        self.chatId = chatId
        self.text = text
        self.content = content
        self.entities = entities
        self.replyTo = replyTo
        self.quoteText = quoteText
        self.topicId = topicId
        self.clientNonce = clientNonce
        self.forwardFromChatId = forwardFromChatId
        self.forwardFromMsgId = forwardFromMsgId
        self.silent = silent
        self.scheduledAtMs = scheduledAtMs
        self.inlineKeyboard = inlineKeyboard
        self.sendAsPeerId = sendAsPeerId
        self.skipServerBotReply = skipServerBotReply
    }
}

public struct FabushiSendMessageResponse: Codable, Sendable, Equatable {
    public let messageId: Int64
    public let pts: UInt64
    public let duplicate: Bool
    public let dateMs: Int64
    public let senderId: UInt64
    public let chatId: Int64
    public let text: String
    public let clientNonce: UInt64?
    public let replyTo: Int64?
    public let status: String

    private enum CodingKeys: String, CodingKey {
        case messageId = "message_id"
        case id
        case pts
        case duplicate
        case deduplicated
        case dateMs = "date_ms"
        case date
        case timestamp
        case senderId = "sender_id"
        case chatId = "chat_id"
        case text
        case clientNonce = "client_nonce"
        case replyTo = "reply_to"
        case status
        case deliveryStatus = "delivery_status"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.messageId = try container.decodeIfPresent(Int64.self, forKey: .messageId)
            ?? container.decodeIfPresent(Int64.self, forKey: .id)
            ?? 0
        self.pts = try container.decodeIfPresent(UInt64.self, forKey: .pts) ?? 0
        self.duplicate = try container.decodeIfPresent(Bool.self, forKey: .duplicate)
            ?? container.decodeIfPresent(Bool.self, forKey: .deduplicated)
            ?? false
        let rawDate = (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .dateMs))
            ?? (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .date))
            ?? (try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .timestamp))
        self.dateMs = FabushiWireDateParser.parseTimestampMs(from: rawDate) ?? FabushiWireDateParser.nowMs()
        self.senderId = try container.decodeIfPresent(UInt64.self, forKey: .senderId) ?? 0
        self.chatId = try container.decodeIfPresent(Int64.self, forKey: .chatId) ?? 0
        self.text = try container.decodeIfPresent(String.self, forKey: .text) ?? ""
        self.clientNonce = try container.decodeIfPresent(UInt64.self, forKey: .clientNonce)
        self.replyTo = try container.decodeIfPresent(Int64.self, forKey: .replyTo)
        self.status = try container.decodeIfPresent(String.self, forKey: .status)
            ?? container.decodeIfPresent(String.self, forKey: .deliveryStatus)
            ?? "sent"
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(messageId, forKey: .messageId)
        try container.encode(pts, forKey: .pts)
        try container.encode(duplicate, forKey: .duplicate)
        try container.encode(dateMs, forKey: .dateMs)
        try container.encode(senderId, forKey: .senderId)
        try container.encode(chatId, forKey: .chatId)
        try container.encode(text, forKey: .text)
        try container.encodeIfPresent(clientNonce, forKey: .clientNonce)
        try container.encodeIfPresent(replyTo, forKey: .replyTo)
        try container.encode(status, forKey: .status)
    }
}

public struct FabushiCreateChatRequest: Codable, Sendable, Equatable {
    public let title: String
    public let chatType: Int?
    public let peerUserId: UInt64?
    public let username: String?
    public let about: String?
    public let isForum: Bool
    public let memberIds: [UInt64]
    public let memberUsernames: [String]
    public let clientNonce: String?

    public enum CodingKeys: String, CodingKey {
        case title
        case chatType = "chat_type"
        case peerUserId = "peer_user_id"
        case username
        case about
        case isForum = "is_forum"
        case memberIds = "member_ids"
        case memberUsernames = "member_usernames"
        case clientNonce = "client_nonce"
    }

    public init(
        title: String,
        chatType: Int? = nil,
        peerUserId: UInt64? = nil,
        username: String? = nil,
        about: String? = nil,
        isForum: Bool = false,
        memberIds: [UInt64] = [],
        memberUsernames: [String] = [],
        clientNonce: String? = nil
    ) {
        self.title = title
        self.chatType = chatType
        self.peerUserId = peerUserId
        self.username = username
        self.about = about
        self.isForum = isForum
        self.memberIds = memberIds
        self.memberUsernames = memberUsernames
        self.clientNonce = clientNonce
    }
}

public struct FabushiDraftRecord: Codable, Sendable, Equatable, Hashable {
    public let chatId: Int64
    public var text: String
    public var entities: [FabushiMessageEntity]
    public var replyToMsgId: Int64?
    public var topicId: Int64?
    public var dateMs: Int64

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case text
        case entities
        case replyToMsgId = "reply_to_msg_id"
        case topicId = "topic_id"
        case dateMs = "date_ms"
    }

    public init(
        chatId: Int64,
        text: String,
        entities: [FabushiMessageEntity] = [],
        replyToMsgId: Int64? = nil,
        topicId: Int64? = nil,
        dateMs: Int64 = FabushiWireDateParser.nowMs()
    ) {
        self.chatId = chatId
        self.text = text
        self.entities = entities
        self.replyToMsgId = replyToMsgId
        self.topicId = topicId
        self.dateMs = dateMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.chatId = try container.decodeIfPresent(Int64.self, forKey: .chatId) ?? 0
        self.text = try container.decodeIfPresent(String.self, forKey: .text) ?? ""
        self.entities = try container.decodeIfPresent([FabushiMessageEntity].self, forKey: .entities) ?? []
        self.replyToMsgId = try container.decodeIfPresent(Int64.self, forKey: .replyToMsgId)
        self.topicId = try container.decodeIfPresent(Int64.self, forKey: .topicId)
        self.dateMs = try container.decodeIfPresent(Int64.self, forKey: .dateMs) ?? 0
    }
}

public struct FabushiPinnedMessageRecord: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: Int64 { messageId }

    public let chatId: Int64
    public let messageId: Int64
    public let pinnedBy: UInt64
    public let pinnedAtMs: Int64
    public let previewText: String

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case messageId = "message_id"
        case pinnedBy = "pinned_by"
        case pinnedAtMs = "pinned_at_ms"
        case previewText = "preview_text"
    }

    public init(chatId: Int64, messageId: Int64, pinnedBy: UInt64 = 0, pinnedAtMs: Int64 = 0, previewText: String = "") {
        self.chatId = chatId
        self.messageId = messageId
        self.pinnedBy = pinnedBy
        self.pinnedAtMs = pinnedAtMs
        self.previewText = previewText
    }
}

// MARK: - Folders & Forum Topics

public enum FabushiBuiltInFolderFilter: String, CaseIterable, Sendable, Equatable, Hashable, Identifiable {
    case all
    case unread
    case bots

    public var id: String { rawValue }
}

public struct FabushiFolder: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: Int64
    public var title: String
    public var emoticon: String?
    public var includeChats: [Int64]
    public var excludeChats: [Int64]
    public var pinnedChats: [Int64]
    public var includeContacts: Bool
    public var includeNonContacts: Bool
    public var includeGroups: Bool
    public var includeChannels: Bool
    public var includeBots: Bool
    public var excludeMuted: Bool
    public var excludeRead: Bool
    public var excludeArchived: Bool
    public var sortOrder: Int

    public enum CodingKeys: String, CodingKey {
        case id
        case folderId = "folder_id"
        case title
        case emoticon
        case includeChats = "include_chats"
        case includedChatIds = "included_chat_ids"
        case excludeChats = "exclude_chats"
        case excludedChatIds = "excluded_chat_ids"
        case pinnedChats = "pinned_chats"
        case pinnedChatIds = "pinned_chat_ids"
        case includeContacts = "include_contacts"
        case includeNonContacts = "include_non_contacts"
        case includeGroups = "include_groups"
        case includeChannels = "include_channels"
        case includeBots = "include_bots"
        case excludeMuted = "exclude_muted"
        case excludeRead = "exclude_read"
        case excludeArchived = "exclude_archived"
        case sortOrder = "sort_order"
    }

    public init(
        id: Int64 = 0,
        title: String,
        emoticon: String? = nil,
        includeChats: [Int64] = [],
        excludeChats: [Int64] = [],
        pinnedChats: [Int64] = [],
        includeContacts: Bool = false,
        includeNonContacts: Bool = false,
        includeGroups: Bool = false,
        includeChannels: Bool = false,
        includeBots: Bool = false,
        excludeMuted: Bool = false,
        excludeRead: Bool = false,
        excludeArchived: Bool = true,
        sortOrder: Int = 0
    ) {
        self.id = id
        self.title = title
        self.emoticon = emoticon
        self.includeChats = includeChats
        self.excludeChats = excludeChats
        self.pinnedChats = pinnedChats
        self.includeContacts = includeContacts
        self.includeNonContacts = includeNonContacts
        self.includeGroups = includeGroups
        self.includeChannels = includeChannels
        self.includeBots = includeBots
        self.excludeMuted = excludeMuted
        self.excludeRead = excludeRead
        self.excludeArchived = excludeArchived
        self.sortOrder = sortOrder
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try container.decodeIfPresent(Int64.self, forKey: .id)
            ?? container.decodeIfPresent(Int64.self, forKey: .folderId)
            ?? 0
        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        self.emoticon = try container.decodeIfPresent(String.self, forKey: .emoticon)
        self.includeChats = try container.decodeIfPresent([Int64].self, forKey: .includeChats)
            ?? container.decodeIfPresent([Int64].self, forKey: .includedChatIds)
            ?? []
        self.excludeChats = try container.decodeIfPresent([Int64].self, forKey: .excludeChats)
            ?? container.decodeIfPresent([Int64].self, forKey: .excludedChatIds)
            ?? []
        self.pinnedChats = try container.decodeIfPresent([Int64].self, forKey: .pinnedChats)
            ?? container.decodeIfPresent([Int64].self, forKey: .pinnedChatIds)
            ?? []
        self.includeContacts = try container.decodeIfPresent(Bool.self, forKey: .includeContacts) ?? false
        self.includeNonContacts = try container.decodeIfPresent(Bool.self, forKey: .includeNonContacts) ?? false
        self.includeGroups = try container.decodeIfPresent(Bool.self, forKey: .includeGroups) ?? false
        self.includeChannels = try container.decodeIfPresent(Bool.self, forKey: .includeChannels) ?? false
        self.includeBots = try container.decodeIfPresent(Bool.self, forKey: .includeBots) ?? false
        self.excludeMuted = try container.decodeIfPresent(Bool.self, forKey: .excludeMuted) ?? false
        self.excludeRead = try container.decodeIfPresent(Bool.self, forKey: .excludeRead) ?? false
        self.excludeArchived = try container.decodeIfPresent(Bool.self, forKey: .excludeArchived) ?? false
        self.sortOrder = try container.decodeIfPresent(Int.self, forKey: .sortOrder) ?? 0
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(id, forKey: .id)
        try container.encode(title, forKey: .title)
        try container.encodeIfPresent(emoticon, forKey: .emoticon)
        try container.encode(includeChats, forKey: .includeChats)
        try container.encode(excludeChats, forKey: .excludeChats)
        try container.encode(pinnedChats, forKey: .pinnedChats)
        try container.encode(includeContacts, forKey: .includeContacts)
        try container.encode(includeNonContacts, forKey: .includeNonContacts)
        try container.encode(includeGroups, forKey: .includeGroups)
        try container.encode(includeChannels, forKey: .includeChannels)
        try container.encode(includeBots, forKey: .includeBots)
        try container.encode(excludeMuted, forKey: .excludeMuted)
        try container.encode(excludeRead, forKey: .excludeRead)
        try container.encode(excludeArchived, forKey: .excludeArchived)
        try container.encode(sortOrder, forKey: .sortOrder)
    }

    public func matches(_ chat: FabushiMessagingChat) -> Bool {
        if excludeChats.contains(chat.chatId) { return false }
        if includeChats.contains(chat.chatId) { return true }
        if excludeArchived && chat.isArchived { return false }
        if excludeMuted && chat.isMuted { return false }
        if excludeRead && chat.unreadCount == 0 { return false }
        if includeBots && chat.isBot { return true }
        if includeGroups && chat.isGroup { return true }
        if includeChannels && chat.isChannel { return true }
        if (includeContacts || includeNonContacts) && !chat.isGroup && !chat.isChannel && !chat.isBot {
            return true
        }
        return !includeContacts && !includeNonContacts && !includeGroups && !includeChannels && !includeBots && includeChats.isEmpty
    }
}

public struct FabushiForumTopic: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: Int64 { topicId }

    public let chatId: Int64
    public let topicId: Int64
    public var title: String
    public var iconColor: UInt32
    public var iconEmojiId: Int64?
    public let creatorId: UInt64
    public var isPinned: Bool
    public var isClosed: Bool
    public var isHidden: Bool
    public var lastMessageId: Int64?
    public var unreadCount: Int
    public let createdAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case topicId = "topic_id"
        case title
        case iconColor = "icon_color"
        case iconEmojiId = "icon_emoji_id"
        case creatorId = "creator_id"
        case isPinned = "is_pinned"
        case isClosed = "is_closed"
        case isHidden = "is_hidden"
        case lastMessageId = "last_message_id"
        case unreadCount = "unread_count"
        case createdAtMs = "created_at_ms"
    }

    public init(
        chatId: Int64,
        topicId: Int64,
        title: String,
        iconColor: UInt32 = 0,
        iconEmojiId: Int64? = nil,
        creatorId: UInt64 = 0,
        isPinned: Bool = false,
        isClosed: Bool = false,
        isHidden: Bool = false,
        lastMessageId: Int64? = nil,
        unreadCount: Int = 0,
        createdAtMs: Int64 = 0
    ) {
        self.chatId = chatId
        self.topicId = topicId
        self.title = title
        self.iconColor = iconColor
        self.iconEmojiId = iconEmojiId
        self.creatorId = creatorId
        self.isPinned = isPinned
        self.isClosed = isClosed
        self.isHidden = isHidden
        self.lastMessageId = lastMessageId
        self.unreadCount = unreadCount
        self.createdAtMs = createdAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.chatId = try container.decodeIfPresent(Int64.self, forKey: .chatId) ?? 0
        self.topicId = try container.decodeIfPresent(Int64.self, forKey: .topicId) ?? 0
        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        self.iconColor = try container.decodeIfPresent(UInt32.self, forKey: .iconColor) ?? 0
        self.iconEmojiId = try container.decodeIfPresent(Int64.self, forKey: .iconEmojiId)
        self.creatorId = try container.decodeIfPresent(UInt64.self, forKey: .creatorId) ?? 0
        self.isPinned = try container.decodeIfPresent(Bool.self, forKey: .isPinned) ?? false
        self.isClosed = try container.decodeIfPresent(Bool.self, forKey: .isClosed) ?? false
        self.isHidden = try container.decodeIfPresent(Bool.self, forKey: .isHidden) ?? false
        self.lastMessageId = try container.decodeIfPresent(Int64.self, forKey: .lastMessageId)
        self.unreadCount = try container.decodeIfPresent(Int.self, forKey: .unreadCount) ?? 0
        self.createdAtMs = try container.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
    }
}

// MARK: - Contacts, Blocklist, Send-As, Members, Invites, Stats & Prestart Bot

public struct FabushiContact: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { userId }

    public let userId: UInt64
    public var firstName: String
    public var lastName: String?
    public var username: String?
    public var phone: String?
    public var isBot: Bool
    public var isMutual: Bool
    public var chatId: Int64?

    public var displayName: String {
        let full = [firstName, lastName ?? ""]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        if !full.isEmpty { return full }
        if let username, !username.isEmpty { return "@\(username)" }
        return "User \(userId)"
    }

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case firstName = "first_name"
        case lastName = "last_name"
        case username
        case phone
        case isBot = "is_bot"
        case isMutual = "is_mutual"
        case chatId = "chat_id"
    }

    public init(
        userId: UInt64,
        firstName: String,
        lastName: String? = nil,
        username: String? = nil,
        phone: String? = nil,
        isBot: Bool = false,
        isMutual: Bool = false,
        chatId: Int64? = nil
    ) {
        self.userId = userId
        self.firstName = firstName
        self.lastName = lastName
        self.username = username
        self.phone = phone
        self.isBot = isBot
        self.isMutual = isMutual
        self.chatId = chatId
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.userId = try container.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        self.firstName = try container.decodeIfPresent(String.self, forKey: .firstName) ?? ""
        self.lastName = try container.decodeIfPresent(String.self, forKey: .lastName)
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.phone = try container.decodeIfPresent(String.self, forKey: .phone)
        self.isBot = try container.decodeIfPresent(Bool.self, forKey: .isBot) ?? false
        self.isMutual = try container.decodeIfPresent(Bool.self, forKey: .isMutual) ?? false
        self.chatId = try container.decodeIfPresent(Int64.self, forKey: .chatId)
    }
}

public struct FabushiAddContactRequest: Codable, Sendable, Equatable {
    public let firstName: String
    public let lastName: String?
    public let username: String?
    public let phone: String?
    public let isBot: Bool
    public let botSystemPrompt: String?

    public enum CodingKeys: String, CodingKey {
        case firstName = "first_name"
        case lastName = "last_name"
        case username
        case phone
        case isBot = "is_bot"
        case botSystemPrompt = "bot_system_prompt"
    }

    public init(
        firstName: String,
        lastName: String? = nil,
        username: String? = nil,
        phone: String? = nil,
        isBot: Bool = false,
        botSystemPrompt: String? = nil
    ) {
        self.firstName = firstName
        self.lastName = lastName
        self.username = username
        self.phone = phone
        self.isBot = isBot
        self.botSystemPrompt = botSystemPrompt
    }
}

public struct FabushiAddContactResponse: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let chatId: Int64
    public let title: String
    public let username: String
    public let isBot: Bool
    public let avatarColor: UInt32?

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case chatId = "chat_id"
        case title
        case username
        case isBot = "is_bot"
        case avatarColor = "avatar_color"
    }
}

public struct FabushiUserProfile: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { userId }

    public let userId: UInt64
    public var username: String?
    public var firstName: String
    public var lastName: String?
    public var bio: String?
    public var phone: String?
    public var avatarMediaId: String?
    public var emojiStatus: String?
    public var isBot: Bool
    public var isVerified: Bool
    public var isPremium: Bool
    public var status: String
    public var lastSeenMs: Int64?

    public var displayName: String {
        let full = [firstName, lastName ?? ""]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        if !full.isEmpty { return full }
        if let username, !username.isEmpty { return "@\(username)" }
        return "User \(userId)"
    }

    private enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case firstName = "first_name"
        case lastName = "last_name"
        case bio
        case phone
        case avatarMediaId = "avatar_media_id"
        case photoFileId = "photo_file_id"
        case emojiStatus = "emoji_status"
        case isBot = "is_bot"
        case isVerified = "is_verified"
        case isPremium = "is_premium"
        case status
        case lastSeenMs = "last_seen_ms"
    }

    public init(
        userId: UInt64,
        username: String? = nil,
        firstName: String = "",
        lastName: String? = nil,
        bio: String? = nil,
        phone: String? = nil,
        avatarMediaId: String? = nil,
        emojiStatus: String? = nil,
        isBot: Bool = false,
        isVerified: Bool = false,
        isPremium: Bool = false,
        status: String = "offline",
        lastSeenMs: Int64? = nil
    ) {
        self.userId = userId
        self.username = username
        self.firstName = firstName
        self.lastName = lastName
        self.bio = bio
        self.phone = phone
        self.avatarMediaId = avatarMediaId
        self.emojiStatus = emojiStatus
        self.isBot = isBot
        self.isVerified = isVerified
        self.isPremium = isPremium
        self.status = status
        self.lastSeenMs = lastSeenMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.userId = try container.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.firstName = try container.decodeIfPresent(String.self, forKey: .firstName) ?? ""
        self.lastName = try container.decodeIfPresent(String.self, forKey: .lastName)
        self.bio = try container.decodeIfPresent(String.self, forKey: .bio)
        self.phone = try container.decodeIfPresent(String.self, forKey: .phone)
        self.avatarMediaId = try container.decodeIfPresent(String.self, forKey: .avatarMediaId)
            ?? container.decodeIfPresent(String.self, forKey: .photoFileId)
        self.emojiStatus = try container.decodeIfPresent(String.self, forKey: .emojiStatus)
        self.isBot = try container.decodeIfPresent(Bool.self, forKey: .isBot) ?? false
        self.isVerified = try container.decodeIfPresent(Bool.self, forKey: .isVerified) ?? false
        self.isPremium = try container.decodeIfPresent(Bool.self, forKey: .isPremium) ?? false
        if let strStatus = try? container.decodeIfPresent(String.self, forKey: .status) {
            self.status = strStatus
        } else {
            self.status = "offline"
        }
        self.lastSeenMs = try container.decodeIfPresent(Int64.self, forKey: .lastSeenMs)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(userId, forKey: .userId)
        try container.encodeIfPresent(username, forKey: .username)
        try container.encode(firstName, forKey: .firstName)
        try container.encodeIfPresent(lastName, forKey: .lastName)
        try container.encodeIfPresent(bio, forKey: .bio)
        try container.encodeIfPresent(phone, forKey: .phone)
        try container.encodeIfPresent(avatarMediaId, forKey: .avatarMediaId)
        try container.encodeIfPresent(emojiStatus, forKey: .emojiStatus)
        try container.encode(isBot, forKey: .isBot)
        try container.encode(isVerified, forKey: .isVerified)
        try container.encode(isPremium, forKey: .isPremium)
        try container.encode(status, forKey: .status)
        try container.encodeIfPresent(lastSeenMs, forKey: .lastSeenMs)
    }
}

public struct FabushiBlockedUser: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { userId }

    public let userId: UInt64
    public let firstName: String
    public let lastName: String?
    public let username: String?
    public let phone: String?
    public let blockedAtMs: Int64

    public var displayName: String {
        let full = [firstName, lastName ?? ""]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        if !full.isEmpty { return full }
        if let username, !username.isEmpty { return "@\(username)" }
        return "User \(userId)"
    }

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case firstName = "first_name"
        case lastName = "last_name"
        case username
        case phone
        case blockedAtMs = "blocked_at_ms"
    }

    public init(userId: UInt64, firstName: String, lastName: String? = nil, username: String? = nil, phone: String? = nil, blockedAtMs: Int64 = 0) {
        self.userId = userId
        self.firstName = firstName
        self.lastName = lastName
        self.username = username
        self.phone = phone
        self.blockedAtMs = blockedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.userId = try container.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        self.firstName = try container.decodeIfPresent(String.self, forKey: .firstName) ?? ""
        self.lastName = try container.decodeIfPresent(String.self, forKey: .lastName)
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.phone = try container.decodeIfPresent(String.self, forKey: .phone)
        self.blockedAtMs = try container.decodeIfPresent(Int64.self, forKey: .blockedAtMs) ?? 0
    }
}

public struct FabushiSendAsPeer: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { peerId }

    public let peerId: UInt64
    public let title: String
    public let username: String?
    public let kind: String
    public let isDefault: Bool

    public enum CodingKeys: String, CodingKey {
        case peerId = "peer_id"
        case title
        case username
        case kind
        case isDefault = "is_default"
    }

    public init(peerId: UInt64, title: String, username: String? = nil, kind: String = "user", isDefault: Bool = false) {
        self.peerId = peerId
        self.title = title
        self.username = username
        self.kind = kind
        self.isDefault = isDefault
    }
}

public struct FabushiChatMember: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { userId }

    public let userId: UInt64
    public let username: String?
    public let firstName: String
    public let lastName: String?
    public let role: String
    public let adminRights: UInt64
    public let adminTitle: String?
    public let joinedAtMs: Int64

    public var displayName: String {
        let full = [firstName, lastName ?? ""]
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        if !full.isEmpty { return full }
        if let username, !username.isEmpty { return "@\(username)" }
        return "User \(userId)"
    }

    public enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case firstName = "first_name"
        case lastName = "last_name"
        case role
        case adminRights = "admin_rights"
        case adminTitle = "admin_title"
        case joinedAtMs = "joined_at_ms"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.userId = try container.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.firstName = try container.decodeIfPresent(String.self, forKey: .firstName) ?? ""
        self.lastName = try container.decodeIfPresent(String.self, forKey: .lastName)
        self.role = try container.decodeIfPresent(String.self, forKey: .role) ?? "member"
        self.adminRights = try container.decodeIfPresent(UInt64.self, forKey: .adminRights) ?? 0
        self.adminTitle = try container.decodeIfPresent(String.self, forKey: .adminTitle)
        self.joinedAtMs = try container.decodeIfPresent(Int64.self, forKey: .joinedAtMs) ?? 0
    }
}

public struct FabushiInviteLink: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { inviteId }

    public let inviteId: String
    public let chatId: Int64
    public let creatorId: UInt64
    public let link: String
    public let code: String
    public let title: String?
    public let usageLimit: UInt32?
    public let usageCount: UInt32
    public let expireDateMs: Int64?
    public let requestNeeded: Bool
    public let isRevoked: Bool
    public let createdAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case inviteId = "invite_id"
        case chatId = "chat_id"
        case creatorId = "creator_id"
        case link
        case code
        case title
        case usageLimit = "usage_limit"
        case usageCount = "usage_count"
        case expireDateMs = "expire_date_ms"
        case requestNeeded = "request_needed"
        case isRevoked = "is_revoked"
        case createdAtMs = "created_at_ms"
    }
}

public struct FabushiDailyStatPoint: Codable, Sendable, Equatable, Hashable {
    public let day: String
    public let timestampMs: Int64
    public let messages: UInt64
    public let reactions: UInt64
    public let activeSenders: UInt64

    public enum CodingKeys: String, CodingKey {
        case day
        case timestampMs = "timestamp_ms"
        case messages
        case reactions
        case activeSenders = "active_senders"
    }
}

public struct FabushiChannelStatistics: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let title: String
    public let memberCount: UInt64
    public let totalMessages: UInt64
    public let messagesLast7d: UInt64
    public let totalReactions: UInt64
    public let activePosters7d: UInt64
    public let growthRatePct: Double
    public let dailySeries: [FabushiDailyStatPoint]

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case title
        case memberCount = "member_count"
        case totalMessages = "total_messages"
        case messagesLast7d = "messages_last_7d"
        case totalReactions = "total_reactions"
        case activePosters7d = "active_posters_7d"
        case growthRatePct = "growth_rate_pct"
        case dailySeries = "daily_series"
    }
}

public struct FabushiBotRegistrySummary: Codable, Sendable, Equatable, Hashable {
    public let botId: UInt64
    public let username: String
    public let displayName: String
    public let descriptionText: String?
    public let about: String?

    private enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case botId = "bot_id"
        case userId = "user_id"
        case id
        case username
        case displayName = "display_name"
        case title
        case name
        case descriptionText = "description"
        case about
    }

    public init(botId: UInt64, username: String, displayName: String, descriptionText: String? = nil, about: String? = nil) {
        self.botId = botId
        self.username = username
        self.displayName = displayName
        self.descriptionText = descriptionText
        self.about = about
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.botId = try container.decodeIfPresent(UInt64.self, forKey: .botUserId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .botId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .userId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .id)
            ?? 0
        self.username = try container.decodeIfPresent(String.self, forKey: .username) ?? ""
        self.displayName = try container.decodeIfPresent(String.self, forKey: .displayName)
            ?? container.decodeIfPresent(String.self, forKey: .title)
            ?? container.decodeIfPresent(String.self, forKey: .name)
            ?? self.username
        self.descriptionText = try container.decodeIfPresent(String.self, forKey: .descriptionText)
        self.about = try container.decodeIfPresent(String.self, forKey: .about)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(botId, forKey: .botId)
        try container.encode(username, forKey: .username)
        try container.encode(displayName, forKey: .displayName)
        try container.encodeIfPresent(descriptionText, forKey: .descriptionText)
        try container.encodeIfPresent(about, forKey: .about)
    }
}

public struct FabushiBotPublicProfile: Codable, Sendable, Equatable {
    public let bot: FabushiBotRegistrySummary
    public let mainAppId: String?
    public let existingChatId: Int64?
    public let hasStartedChat: Bool
    public let allowsWriteToPm: Bool
    public let isBlocked: Bool
    public let isMuted: Bool

    private enum CodingKeys: String, CodingKey {
        case bot
        case mainApp = "main_app"
        case existingChatId = "existing_chat_id"
        case hasStartedChat = "has_started_chat"
        case allowsWriteToPm = "allows_write_to_pm"
        case isBlocked = "is_blocked"
        case isMuted = "is_muted"
    }

    public init(
        bot: FabushiBotRegistrySummary,
        mainAppId: String? = nil,
        existingChatId: Int64? = nil,
        hasStartedChat: Bool = false,
        allowsWriteToPm: Bool = false,
        isBlocked: Bool = false,
        isMuted: Bool = false
    ) {
        self.bot = bot
        self.mainAppId = mainAppId
        self.existingChatId = existingChatId
        self.hasStartedChat = hasStartedChat
        self.allowsWriteToPm = allowsWriteToPm
        self.isBlocked = isBlocked
        self.isMuted = isMuted
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.bot = try container.decode(FabushiBotRegistrySummary.self, forKey: .bot)
        if let mainApp = try? container.decodeIfPresent(FabushiJSONValue.self, forKey: .mainApp) {
            self.mainAppId = mainApp["app_id"]?.stringValue ?? mainApp["short_name"]?.stringValue ?? mainApp["id"]?.stringValue
        } else {
            self.mainAppId = nil
        }
        self.existingChatId = try container.decodeIfPresent(Int64.self, forKey: .existingChatId)
        self.hasStartedChat = try container.decodeIfPresent(Bool.self, forKey: .hasStartedChat) ?? false
        self.allowsWriteToPm = try container.decodeIfPresent(Bool.self, forKey: .allowsWriteToPm) ?? false
        self.isBlocked = try container.decodeIfPresent(Bool.self, forKey: .isBlocked) ?? false
        self.isMuted = try container.decodeIfPresent(Bool.self, forKey: .isMuted) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(bot, forKey: .bot)
        try container.encodeIfPresent(existingChatId, forKey: .existingChatId)
        try container.encode(hasStartedChat, forKey: .hasStartedChat)
        try container.encode(allowsWriteToPm, forKey: .allowsWriteToPm)
        try container.encode(isBlocked, forKey: .isBlocked)
        try container.encode(isMuted, forKey: .isMuted)
    }
}

public struct FabushiCanBotSendResponse: Codable, Sendable, Equatable {
    public let canSend: Bool
    public let allowsWriteToPm: Bool?

    private enum CodingKeys: String, CodingKey {
        case canSend = "can_send"
        case allowed
        case allowsWriteToPm = "allows_write_to_pm"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.allowsWriteToPm = try container.decodeIfPresent(Bool.self, forKey: .allowsWriteToPm)
        self.canSend = try container.decodeIfPresent(Bool.self, forKey: .canSend)
            ?? container.decodeIfPresent(Bool.self, forKey: .allowed)
            ?? self.allowsWriteToPm
            ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(canSend, forKey: .canSend)
        try container.encodeIfPresent(allowsWriteToPm, forKey: .allowsWriteToPm)
    }
}

public struct FabushiAllowBotSendResponse: Codable, Sendable, Equatable {
    public let ok: Bool
    public let chatId: Int64?
    public let serviceMessageId: Int64?
    public let serviceText: String?

    public enum CodingKeys: String, CodingKey {
        case ok
        case chatId = "chat_id"
        case serviceMessageId = "service_message_id"
        case serviceText = "service_text"
    }
}

// MARK: - Outbox & PTS Sync Models

public enum FabushiOutboxStatus: String, Codable, Sendable, Equatable, Hashable {
    case pending = "Pending"
    case inFlight = "InFlight"
    case delivered = "Delivered"
    case failed = "Failed"

    public init(wireValue: String) {
        switch wireValue.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "inflight", "in_flight", "sending": self = .inFlight
        case "delivered", "sent": self = .delivered
        case "failed", "error": self = .failed
        default: self = .pending
        }
    }
}

public struct FabushiOutboxEntry: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { clientNonce }

    public let clientNonce: UInt64
    public let chatId: Int64
    public let payloadJSON: String
    public let textPreview: String
    public var status: FabushiOutboxStatus
    public var retryCount: Int
    public var nextRetryAtMs: Int64
    public var lastError: String?
    public var serverMessageId: Int64?
    public let createdAtMs: Int64
    public var updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case clientNonce = "client_nonce"
        case chatId = "chat_id"
        case payloadJSON = "payload_json"
        case textPreview = "text_preview"
        case status
        case retryCount = "retry_count"
        case nextRetryAtMs = "next_retry_at_ms"
        case lastError = "last_error"
        case serverMessageId = "server_message_id"
        case createdAtMs = "created_at_ms"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        clientNonce: UInt64,
        chatId: Int64,
        payloadJSON: String = "{}",
        textPreview: String,
        status: FabushiOutboxStatus,
        retryCount: Int = 0,
        nextRetryAtMs: Int64 = 0,
        lastError: String? = nil,
        serverMessageId: Int64? = nil,
        createdAtMs: Int64 = FabushiWireDateParser.nowMs(),
        updatedAtMs: Int64 = FabushiWireDateParser.nowMs()
    ) {
        self.clientNonce = clientNonce
        self.chatId = chatId
        self.payloadJSON = payloadJSON
        self.textPreview = textPreview
        self.status = status
        self.retryCount = retryCount
        self.nextRetryAtMs = nextRetryAtMs
        self.lastError = lastError
        self.serverMessageId = serverMessageId
        self.createdAtMs = createdAtMs
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.clientNonce = try container.decodeIfPresent(UInt64.self, forKey: .clientNonce) ?? 0
        self.chatId = try container.decodeIfPresent(Int64.self, forKey: .chatId) ?? 0
        self.payloadJSON = try container.decodeIfPresent(String.self, forKey: .payloadJSON) ?? "{}"
        self.textPreview = try container.decodeIfPresent(String.self, forKey: .textPreview) ?? ""
        let rawStatus = try container.decodeIfPresent(String.self, forKey: .status) ?? "Pending"
        self.status = FabushiOutboxStatus(wireValue: rawStatus)
        self.retryCount = try container.decodeIfPresent(Int.self, forKey: .retryCount) ?? 0
        self.nextRetryAtMs = try container.decodeIfPresent(Int64.self, forKey: .nextRetryAtMs) ?? 0
        self.lastError = try container.decodeIfPresent(String.self, forKey: .lastError)
        self.serverMessageId = try container.decodeIfPresent(Int64.self, forKey: .serverMessageId)
        self.createdAtMs = try container.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        self.updatedAtMs = try container.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct FabushiSyncDifferenceResponse: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let newMessages: [FabushiMessageRecord]
    public let pts: UInt64
    public let currentPts: UInt64
    public let hasMore: Bool
    public let snapshotRequired: Bool

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case newMessages = "new_messages"
        case pts
        case currentPts = "current_pts"
        case hasMore = "has_more"
        case snapshotRequired = "snapshot_required"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.chatId = try container.decodeIfPresent(Int64.self, forKey: .chatId) ?? 0
        self.newMessages = try container.decodeIfPresent([FabushiMessageRecord].self, forKey: .newMessages) ?? []
        self.pts = try container.decodeIfPresent(UInt64.self, forKey: .pts) ?? 0
        self.currentPts = try container.decodeIfPresent(UInt64.self, forKey: .currentPts) ?? self.pts
        self.hasMore = try container.decodeIfPresent(Bool.self, forKey: .hasMore) ?? false
        self.snapshotRequired = try container.decodeIfPresent(Bool.self, forKey: .snapshotRequired) ?? false
    }
}

public struct FabushiPtsEvaluationResult: Codable, Sendable, Equatable {
    public let outcome: String
    public let lastPts: UInt64?
    public let lastSeen: UInt64?
    public let incoming: UInt64?
    public let missingStart: UInt64?
    public let missingEnd: UInt64?

    public enum CodingKeys: String, CodingKey {
        case outcome
        case lastPts = "last_pts"
        case lastSeen = "last_seen"
        case incoming
        case missingStart = "missing_start"
        case missingEnd = "missing_end"
    }
}

/// Per-account disk hydration snapshot (`D13-01` / `D13-02`).
public struct FabushiSyncSnapshot: Codable, Sendable, Equatable {
    public static let schemaVersion: Int = 1

    public var version: Int
    public var savedAtMs: Int64
    public var lastActiveChatId: Int64?
    public var chats: [FabushiMessagingChat]
    public var folders: [FabushiFolder]
    public var contacts: [FabushiContact]
    public var messagesByChat: [Int64: [FabushiMessageRecord]]
    public var draftsByChat: [Int64: FabushiDraftRecord]
    public var ptsByChat: [Int64: UInt64]

    public init(
        version: Int = schemaVersion,
        savedAtMs: Int64 = FabushiWireDateParser.nowMs(),
        lastActiveChatId: Int64? = nil,
        chats: [FabushiMessagingChat] = [],
        folders: [FabushiFolder] = [],
        contacts: [FabushiContact] = [],
        messagesByChat: [Int64: [FabushiMessageRecord]] = [:],
        draftsByChat: [Int64: FabushiDraftRecord] = [:],
        ptsByChat: [Int64: UInt64] = [:]
    ) {
        self.version = version
        self.savedAtMs = savedAtMs
        self.lastActiveChatId = lastActiveChatId
        self.chats = chats
        self.folders = folders
        self.contacts = contacts
        self.messagesByChat = messagesByChat
        self.draftsByChat = draftsByChat
        self.ptsByChat = ptsByChat
    }
}

// MARK: - Unified Search Models (`#18`, Honest `BLK-08`)

/// Strictly the 8 server/local backed search categories. Community is intentionally excluded (`BLK-08`).
public enum FabushiSearchScope: String, CaseIterable, Codable, Sendable, Equatable, Hashable, Identifiable {
    case all
    case chats
    case messages
    case media
    case links
    case files
    case bots
    case miniApps = "mini_apps"

    public var id: String { rawValue }

    public var stringKey: FabushiMessagingStrings.Key {
        switch self {
        case .all: return .searchScopeAll
        case .chats: return .searchScopeChats
        case .messages: return .searchScopeMessages
        case .media: return .searchScopeMedia
        case .links: return .searchScopeLinks
        case .files: return .searchScopeFiles
        case .bots: return .searchScopeBots
        case .miniApps: return .searchScopeMiniApps
        }
    }

    public var remoteMessageFilter: String? {
        switch self {
        case .media: return "media"
        case .links: return "links"
        case .files: return "files"
        default: return nil
        }
    }

    public var remoteCatalogCategory: String? {
        switch self {
        case .all: return nil
        case .chats: return "chats"
        case .bots: return "bots"
        case .miniApps: return "apps"
        default: return nil
        }
    }
}

public enum FabushiSearchResultKind: String, Codable, Sendable, Equatable, Hashable {
    case chat
    case message
    case media
    case link
    case file
    case bot
    case miniApp = "mini_app"
    case sessionMemory = "session_memory"
}

public struct FabushiSearchResult: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: String
    public let kind: FabushiSearchResultKind
    public let chatId: Int64?
    public let messageId: Int64?
    public let botUserId: UInt64?
    public let miniAppId: String?
    public let title: String
    public let snippet: String
    public let timestampMs: Int64?
    public let isOfflineLocalHit: Bool

    public init(
        id: String,
        kind: FabushiSearchResultKind,
        chatId: Int64? = nil,
        messageId: Int64? = nil,
        botUserId: UInt64? = nil,
        miniAppId: String? = nil,
        title: String,
        snippet: String,
        timestampMs: Int64? = nil,
        isOfflineLocalHit: Bool = false
    ) {
        self.id = id
        self.kind = kind
        self.chatId = chatId
        self.messageId = messageId
        self.botUserId = botUserId
        self.miniAppId = miniAppId
        self.title = title
        self.snippet = snippet
        self.timestampMs = timestampMs
        self.isOfflineLocalHit = isOfflineLocalHit
    }
}

// MARK: - Slash Commands, Context Badges, AI Model Catalog (`D10-09`), Bots & Computers

public struct FabushiSlashCommand: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { slashName }

    public let name: String
    public let slashName: String
    public let aliases: [String]
    public let syntax: String
    public let category: String
    public let descriptionText: String
    public let acceptsArgs: Bool

    public enum CodingKeys: String, CodingKey {
        case name
        case slashName = "slash_name"
        case aliases
        case syntax
        case category
        case descriptionText = "description"
        case acceptsArgs = "accepts_args"
    }

    public init(
        name: String,
        slashName: String,
        aliases: [String] = [],
        syntax: String = "",
        category: String = "session",
        descriptionText: String = "",
        acceptsArgs: Bool = false
    ) {
        self.name = name
        self.slashName = slashName
        self.aliases = aliases
        self.syntax = syntax
        self.category = category
        self.descriptionText = descriptionText
        self.acceptsArgs = acceptsArgs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let decodedName = try container.decodeIfPresent(String.self, forKey: .name) ?? ""
        self.name = decodedName
        self.slashName = try container.decodeIfPresent(String.self, forKey: .slashName) ?? "/\(decodedName)"
        self.aliases = try container.decodeIfPresent([String].self, forKey: .aliases) ?? []
        self.syntax = try container.decodeIfPresent(String.self, forKey: .syntax) ?? self.slashName
        self.category = try container.decodeIfPresent(String.self, forKey: .category) ?? "session"
        self.descriptionText = try container.decodeIfPresent(String.self, forKey: .descriptionText) ?? ""
        self.acceptsArgs = try container.decodeIfPresent(Bool.self, forKey: .acceptsArgs) ?? false
    }
}

public struct FabushiCommandExecutionResponse: Codable, Sendable, Equatable {
    public let handled: Bool
    public let command: String
    public let sessionId: String?
    public let output: String
    public let replyText: String?
    public let inlineKeyboard: [[FabushiInlineKeyboardButton]]
    public let effects: [FabushiJSONValue]
    public let data: FabushiJSONValue?

    public enum CodingKeys: String, CodingKey {
        case handled
        case command
        case sessionId = "session_id"
        case output
        case replyText = "reply_text"
        case inlineKeyboard = "inline_keyboard"
        case effects
        case data
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.handled = try container.decodeIfPresent(Bool.self, forKey: .handled) ?? false
        self.command = try container.decodeIfPresent(String.self, forKey: .command) ?? ""
        self.sessionId = try container.decodeIfPresent(String.self, forKey: .sessionId)
        self.output = try container.decodeIfPresent(String.self, forKey: .output) ?? ""
        self.replyText = try container.decodeIfPresent(String.self, forKey: .replyText)
        self.inlineKeyboard = try container.decodeIfPresent([[FabushiInlineKeyboardButton]].self, forKey: .inlineKeyboard) ?? []
        self.effects = try container.decodeIfPresent([FabushiJSONValue].self, forKey: .effects) ?? []
        self.data = try container.decodeIfPresent(FabushiJSONValue.self, forKey: .data)
    }
}

public enum FabushiContextBadgeKind: String, CaseIterable, Codable, Sendable, Equatable, Hashable {
    case file = "@file"
    case directory = "@dir"
    case diff = "@diff"
    case git = "@git:"
    case url = "@url"
}

public struct FabushiContextBadge: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: String
    public let kind: FabushiContextBadgeKind
    public let label: String
    public let value: String

    public init(id: String = UUID().uuidString, kind: FabushiContextBadgeKind, label: String, value: String) {
        self.id = id
        self.kind = kind
        self.label = label
        self.value = value
    }

    public var formattedToken: String {
        switch kind {
        case .git:
            return "@git:\(value)"
        default:
            return "\(kind.rawValue) \(value)"
        }
    }
}

/// Dynamic AI model catalog strictly built from live `magpie.state` (`D10-09`).
/// ZERO hardcoded model defaults.
public struct FabushiAiModelCatalog: Codable, Sendable, Equatable {
    public struct Entry: Identifiable, Codable, Sendable, Equatable, Hashable {
        public var id: String { modelId }
        public let modelId: String
        public let providerId: String
        public let displayName: String

        public init(modelId: String, providerId: String, displayName: String? = nil) {
            self.modelId = modelId
            self.providerId = providerId
            self.displayName = displayName ?? modelId
        }
    }

    public static let followSettingsSentinels: Set<String> = [
        "",
        "default",
        "默认",
        "默认模型",
        "auto",
        "hermes-local-v1",
        "local_hermes/hermes-local-v1",
    ]

    public let activeProvider: String?
    public let activeModel: String?
    public let models: [Entry]
    public let routingGroups: [String]
    public let hasConfiguredProvider: Bool

    public init(
        activeProvider: String? = nil,
        activeModel: String? = nil,
        models: [Entry] = [],
        routingGroups: [String] = [],
        hasConfiguredProvider: Bool = false
    ) {
        self.activeProvider = activeProvider
        self.activeModel = activeModel
        self.models = models
        self.routingGroups = routingGroups
        self.hasConfiguredProvider = hasConfiguredProvider
    }

    /// Normalizes placeholder sentinels back to `""` (Follow Settings).
    public static func normalizePreferredModel(_ raw: String?) -> String {
        let trimmed = (raw ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if followSettingsSentinels.contains(trimmed.lowercased()) || followSettingsSentinels.contains(trimmed) {
            return ""
        }
        return trimmed
    }

    /// Returns true when a saved non-empty model override is not present in the live Settings catalog.
    public func isMismatchedOverride(_ preferredModel: String?) -> Bool {
        let normalized = Self.normalizePreferredModel(preferredModel)
        guard !normalized.isEmpty else { return false }
        let known = Set(models.map(\.modelId) + routingGroups + [activeModel].compactMap { $0 })
        return !known.contains(normalized)
    }

    /// Builds a live catalog exclusively from `magpie.state` JSON (ZERO hardcoded fallbacks).
    public static func fromMagpieStateJSON(_ json: FabushiJSONValue) -> FabushiAiModelCatalog {
        let activeProvider = json["active_provider"]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines)
        let activeModel = json["active_model"]?.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines)

        var seen = Set<String>()
        var entries: [Entry] = []

        func appendEntry(modelId: String, providerId: String, displayName: String? = nil) {
            let cleanModel = modelId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !cleanModel.isEmpty, seen.insert(cleanModel).inserted else { return }
            entries.append(Entry(modelId: cleanModel, providerId: providerId, displayName: displayName))
        }

        if let providers = json["providers"]?.arrayValue {
            for provider in providers {
                let providerId = provider["id"]?.stringValue
                    ?? provider["provider"]?.stringValue
                    ?? provider["name"]?.stringValue
                    ?? ""
                let enabled = provider["enabled"]?.boolValue ?? true
                guard enabled else { continue }
                if let modelList = provider["models"]?.arrayValue {
                    for model in modelList {
                        if let idStr = model.stringValue {
                            appendEntry(modelId: idStr, providerId: providerId)
                        } else if let idStr = model["id"]?.stringValue ?? model["model_id"]?.stringValue {
                            appendEntry(modelId: idStr, providerId: providerId, displayName: model["name"]?.stringValue)
                        }
                    }
                }
                if let defaultModel = provider["default_model"]?.stringValue {
                    appendEntry(modelId: defaultModel, providerId: providerId)
                }
            }
        }

        if let subscriptions = json["subscriptions"]?.arrayValue {
            for sub in subscriptions {
                let providerId = sub["provider"]?.stringValue ?? sub["id"]?.stringValue ?? "subscription"
                if let modelId = sub["model"]?.stringValue ?? sub["default_model"]?.stringValue {
                    appendEntry(modelId: modelId, providerId: providerId)
                }
                if let modelsArr = sub["models"]?.arrayValue {
                    for m in modelsArr {
                        if let mStr = m.stringValue {
                            appendEntry(modelId: mStr, providerId: providerId)
                        }
                    }
                }
            }
        }

        var groups: [String] = []
        let rawGroups = json["routing_groups"]?.arrayValue ?? json["groups"]?.arrayValue ?? []
        for group in rawGroups {
            if let name = group["name"]?.stringValue ?? group["id"]?.stringValue, !name.isEmpty {
                groups.append(name)
            }
        }

        if let activeModel, !activeModel.isEmpty {
            appendEntry(modelId: activeModel, providerId: activeProvider ?? "active")
        }

        let hasProvider = !entries.isEmpty || !(activeProvider ?? "").isEmpty || !(activeModel ?? "").isEmpty
        return FabushiAiModelCatalog(
            activeProvider: (activeProvider?.isEmpty == true) ? nil : activeProvider,
            activeModel: (activeModel?.isEmpty == true) ? nil : activeModel,
            models: entries,
            routingGroups: groups,
            hasConfiguredProvider: hasProvider
        )
    }
}

public struct FabushiBotConfig: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: UInt64 { botId }

    public let botId: UInt64
    public var displayName: String
    public var username: String?
    public var descriptionText: String?
    public var avatarShape: String?
    public var avatarColor: String?
    public var preferredModel: String?
    public var thinkingLevel: String?
    public var approvalMode: String?
    public var toolsetPreset: String?
    public var notifyOnAgentUpdates: Bool
    public var hiddenFromSidebar: Bool
    public var updatedAtMs: Int64

    private enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case botId = "bot_id"
        case userId = "user_id"
        case id
        case displayName = "display_name"
        case title
        case username
        case descriptionText = "description"
        case avatarShape = "avatar_shape"
        case avatarColor = "avatar_color"
        case preferredModel = "preferred_model"
        case thinkingLevel = "thinking_level"
        case approvalMode = "approval_mode"
        case toolsetPreset = "toolset_preset"
        case notifyOnAgentUpdates = "notify_on_agent_updates"
        case hiddenFromSidebar = "hidden_from_sidebar"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        botId: UInt64,
        displayName: String,
        username: String? = nil,
        descriptionText: String? = nil,
        avatarShape: String? = nil,
        avatarColor: String? = nil,
        preferredModel: String? = nil,
        thinkingLevel: String? = nil,
        approvalMode: String? = nil,
        toolsetPreset: String? = nil,
        notifyOnAgentUpdates: Bool = true,
        hiddenFromSidebar: Bool = false,
        updatedAtMs: Int64 = 0
    ) {
        self.botId = botId
        self.displayName = displayName
        self.username = username
        self.descriptionText = descriptionText
        self.avatarShape = avatarShape
        self.avatarColor = avatarColor
        self.preferredModel = preferredModel
        self.thinkingLevel = thinkingLevel
        self.approvalMode = approvalMode
        self.toolsetPreset = toolsetPreset
        self.notifyOnAgentUpdates = notifyOnAgentUpdates
        self.hiddenFromSidebar = hiddenFromSidebar
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.botId = try container.decodeIfPresent(UInt64.self, forKey: .botUserId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .botId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .userId)
            ?? container.decodeIfPresent(UInt64.self, forKey: .id)
            ?? 0
        self.displayName = try container.decodeIfPresent(String.self, forKey: .displayName)
            ?? container.decodeIfPresent(String.self, forKey: .title)
            ?? ""
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.descriptionText = try container.decodeIfPresent(String.self, forKey: .descriptionText)
        self.avatarShape = try container.decodeIfPresent(String.self, forKey: .avatarShape)
        self.avatarColor = try container.decodeIfPresent(String.self, forKey: .avatarColor)
        self.preferredModel = try container.decodeIfPresent(String.self, forKey: .preferredModel)
        self.thinkingLevel = try container.decodeIfPresent(String.self, forKey: .thinkingLevel)
        self.approvalMode = try container.decodeIfPresent(String.self, forKey: .approvalMode)
        self.toolsetPreset = try container.decodeIfPresent(String.self, forKey: .toolsetPreset)
        self.notifyOnAgentUpdates = try container.decodeIfPresent(Bool.self, forKey: .notifyOnAgentUpdates) ?? true
        self.hiddenFromSidebar = try container.decodeIfPresent(Bool.self, forKey: .hiddenFromSidebar) ?? false
        self.updatedAtMs = try container.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(botId, forKey: .botId)
        try container.encode(displayName, forKey: .displayName)
        try container.encodeIfPresent(username, forKey: .username)
        try container.encodeIfPresent(descriptionText, forKey: .descriptionText)
        try container.encodeIfPresent(avatarShape, forKey: .avatarShape)
        try container.encodeIfPresent(avatarColor, forKey: .avatarColor)
        try container.encodeIfPresent(preferredModel, forKey: .preferredModel)
        try container.encodeIfPresent(thinkingLevel, forKey: .thinkingLevel)
        try container.encodeIfPresent(approvalMode, forKey: .approvalMode)
        try container.encodeIfPresent(toolsetPreset, forKey: .toolsetPreset)
        try container.encode(notifyOnAgentUpdates, forKey: .notifyOnAgentUpdates)
        try container.encode(hiddenFromSidebar, forKey: .hiddenFromSidebar)
        try container.encode(updatedAtMs, forKey: .updatedAtMs)
    }
}

public struct FabushiBotComputer: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { nodeId }

    public let nodeId: String
    public let kind: String
    public let displayName: String
    public let ipAddress: String?
    public let osName: String?
    public let online: Bool
    public let runPolicy: String?
    public let boundBotIds: [UInt64]
    public let tierBadge: String?

    public enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case kind
        case displayName = "display_name"
        case ipAddress = "ip_address"
        case osName = "os_name"
        case online
        case runPolicy = "run_policy"
        case boundBotIds = "bound_bot_ids"
        case tierBadge = "tier_badge"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.nodeId = try container.decodeIfPresent(String.self, forKey: .nodeId) ?? ""
        self.kind = try container.decodeIfPresent(String.self, forKey: .kind) ?? "desktop"
        self.displayName = try container.decodeIfPresent(String.self, forKey: .displayName) ?? self.nodeId
        self.ipAddress = try container.decodeIfPresent(String.self, forKey: .ipAddress)
        self.osName = try container.decodeIfPresent(String.self, forKey: .osName)
        self.online = try container.decodeIfPresent(Bool.self, forKey: .online) ?? false
        self.runPolicy = try container.decodeIfPresent(String.self, forKey: .runPolicy)
        self.boundBotIds = try container.decodeIfPresent([UInt64].self, forKey: .boundBotIds) ?? []
        self.tierBadge = try container.decodeIfPresent(String.self, forKey: .tierBadge)
    }
}

public struct FabushiBotComputerProfile: Codable, Sendable, Equatable {
    public let botId: UInt64
    public var boundNodeId: String?
    public var personaPrompt: String
    public var preferredModel: String
    public var approvalMode: String
    public var allowCrossDeviceControl: Bool
    public var allowCloudSelfConfig: Bool
    public var updatedAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case botId = "bot_id"
        case boundNodeId = "bound_node_id"
        case personaPrompt = "persona_prompt"
        case preferredModel = "preferred_model"
        case approvalMode = "approval_mode"
        case allowCrossDeviceControl = "allow_cross_device_control"
        case allowCloudSelfConfig = "allow_cloud_self_config"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        botId: UInt64,
        boundNodeId: String? = nil,
        personaPrompt: String = "",
        preferredModel: String = "",
        approvalMode: String = "ask",
        allowCrossDeviceControl: Bool = false,
        allowCloudSelfConfig: Bool = false,
        updatedAtMs: Int64 = 0
    ) {
        self.botId = botId
        self.boundNodeId = boundNodeId
        self.personaPrompt = personaPrompt
        self.preferredModel = preferredModel
        self.approvalMode = approvalMode
        self.allowCrossDeviceControl = allowCrossDeviceControl
        self.allowCloudSelfConfig = allowCloudSelfConfig
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.botId = try container.decodeIfPresent(UInt64.self, forKey: .botId) ?? 0
        self.boundNodeId = try container.decodeIfPresent(String.self, forKey: .boundNodeId)
        self.personaPrompt = try container.decodeIfPresent(String.self, forKey: .personaPrompt) ?? ""
        self.preferredModel = try container.decodeIfPresent(String.self, forKey: .preferredModel) ?? ""
        self.approvalMode = try container.decodeIfPresent(String.self, forKey: .approvalMode) ?? "ask"
        self.allowCrossDeviceControl = try container.decodeIfPresent(Bool.self, forKey: .allowCrossDeviceControl) ?? false
        self.allowCloudSelfConfig = try container.decodeIfPresent(Bool.self, forKey: .allowCloudSelfConfig) ?? false
        self.updatedAtMs = try container.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct FabushiApprovalItem: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: String
    public let sessionId: String?
    public let command: String
    public let riskLevel: String
    public let patternName: String?
    public let status: String
    public let decidedBy: String?
    public let createdAt: String?

    public enum CodingKeys: String, CodingKey {
        case id
        case sessionId = "session_id"
        case command
        case riskLevel = "risk_level"
        case patternName = "pattern_name"
        case status
        case decidedBy = "decided_by"
        case createdAt = "created_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.id = try container.decodeIfPresent(String.self, forKey: .id) ?? UUID().uuidString
        self.sessionId = try container.decodeIfPresent(String.self, forKey: .sessionId)
        self.command = try container.decodeIfPresent(String.self, forKey: .command) ?? ""
        self.riskLevel = try container.decodeIfPresent(String.self, forKey: .riskLevel) ?? "medium"
        self.patternName = try container.decodeIfPresent(String.self, forKey: .patternName)
        self.status = try container.decodeIfPresent(String.self, forKey: .status) ?? "pending"
        self.decidedBy = try container.decodeIfPresent(String.self, forKey: .decidedBy)
        self.createdAt = try container.decodeIfPresent(String.self, forKey: .createdAt)
    }
}

// MARK: - Bot Activity (`D14-04`) & Compose Recipient

public struct FabushiBotActivityState: Codable, Sendable, Equatable, Hashable {
    public let chatId: Int64
    public let botUserId: Int64
    public let state: String
    public let detail: String?
    public let updatedAtMs: Int64

    public var isActive: Bool {
        let lower = state.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return !lower.isEmpty && lower != "idle" && lower != "done" && lower != "completed" && lower != "stopped"
    }

    public init(chatId: Int64, botUserId: Int64, state: String, detail: String? = nil, updatedAtMs: Int64 = FabushiWireDateParser.nowMs()) {
        self.chatId = chatId
        self.botUserId = botUserId
        self.state = state
        self.detail = detail
        self.updatedAtMs = updatedAtMs
    }
}

public struct FabushiComposeRecipient: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { "\(isBot ? "bot" : "user"):\(userId):\(username ?? "")" }

    public let userId: UInt64
    public let displayName: String
    public let username: String?
    public let isBot: Bool
    public let subtitle: String?

    public init(userId: UInt64, displayName: String, username: String? = nil, isBot: Bool = false, subtitle: String? = nil) {
        self.userId = userId
        self.displayName = displayName
        self.username = username
        self.isBot = isBot
        self.subtitle = subtitle
    }
}

// MARK: - E2EE, Stories, Stickers, GIFs, Media & Instant View Models

public struct FabushiE2EEIdentity: Codable, Sendable, Equatable {
    public let x25519PubB64: String
    public let ed25519PubB64: String
    public let deviceId: String

    public enum CodingKeys: String, CodingKey {
        case x25519PubB64 = "x25519_pub_b64"
        case ed25519PubB64 = "ed25519_pub_b64"
        case deviceId = "device_id"
    }
}

public struct FabushiE2EESafetyInfo: Codable, Sendable, Equatable, Hashable {
    public let chatId: Int64
    public let peerTitle: String
    public let localIdentityKeyB64: String
    public let remoteIdentityKeyB64: String?
    public let safetyEmojis: String
    public let qrPayload: String
    public var isVerified: Bool
    public let peerEnrolledOnServer: Bool

    public init(
        chatId: Int64,
        peerTitle: String,
        localIdentityKeyB64: String,
        remoteIdentityKeyB64: String?,
        safetyEmojis: String,
        qrPayload: String,
        isVerified: Bool = false,
        peerEnrolledOnServer: Bool = false
    ) {
        self.chatId = chatId
        self.peerTitle = peerTitle
        self.localIdentityKeyB64 = localIdentityKeyB64
        self.remoteIdentityKeyB64 = remoteIdentityKeyB64
        self.safetyEmojis = safetyEmojis
        self.qrPayload = qrPayload
        self.isVerified = isVerified
        self.peerEnrolledOnServer = peerEnrolledOnServer
    }
}

public struct FabushiStoryItem: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { storyId }

    public let storyId: String
    public let userId: UInt64
    public let username: String?
    public let firstName: String
    public let mediaId: String?
    public let mediaType: String
    public let caption: String?
    public let privacy: String
    public let viewsCount: Int
    public var isViewed: Bool
    public let createdAtMs: Int64
    public let expiresAtMs: Int64

    public enum CodingKeys: String, CodingKey {
        case storyId = "story_id"
        case userId = "user_id"
        case username
        case firstName = "first_name"
        case mediaId = "media_id"
        case mediaType = "media_type"
        case caption
        case privacy
        case viewsCount = "views_count"
        case isViewed = "is_viewed"
        case createdAtMs = "created_at_ms"
        case expiresAtMs = "expires_at_ms"
    }

    public init(
        storyId: String,
        userId: UInt64,
        username: String? = nil,
        firstName: String,
        mediaId: String? = nil,
        mediaType: String = "photo",
        caption: String? = nil,
        privacy: String = "everyone",
        viewsCount: Int = 0,
        isViewed: Bool = false,
        createdAtMs: Int64 = 0,
        expiresAtMs: Int64 = 0
    ) {
        self.storyId = storyId
        self.userId = userId
        self.username = username
        self.firstName = firstName
        self.mediaId = mediaId
        self.mediaType = mediaType
        self.caption = caption
        self.privacy = privacy
        self.viewsCount = viewsCount
        self.isViewed = isViewed
        self.createdAtMs = createdAtMs
        self.expiresAtMs = expiresAtMs
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.storyId = try container.decodeIfPresent(String.self, forKey: .storyId) ?? UUID().uuidString
        self.userId = try container.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        self.username = try container.decodeIfPresent(String.self, forKey: .username)
        self.firstName = try container.decodeIfPresent(String.self, forKey: .firstName) ?? ""
        self.mediaId = try container.decodeIfPresent(String.self, forKey: .mediaId)
        self.mediaType = try container.decodeIfPresent(String.self, forKey: .mediaType) ?? "photo"
        self.caption = try container.decodeIfPresent(String.self, forKey: .caption)
        self.privacy = try container.decodeIfPresent(String.self, forKey: .privacy) ?? "everyone"
        self.viewsCount = try container.decodeIfPresent(Int.self, forKey: .viewsCount) ?? 0
        self.isViewed = try container.decodeIfPresent(Bool.self, forKey: .isViewed) ?? false
        self.createdAtMs = try container.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
        self.expiresAtMs = try container.decodeIfPresent(Int64.self, forKey: .expiresAtMs) ?? 0
    }
}

public struct FabushiStickerItem: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { fileId }

    public let fileId: String
    public let emoji: String
    public let mimeType: String?
    public let width: UInt32?
    public let height: UInt32?

    public enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case emoji
        case mimeType = "mime_type"
        case width
        case height
    }
}

public struct FabushiStickerPack: Identifiable, Codable, Sendable, Equatable, Hashable {
    public var id: String { setId }

    public let setId: String
    public let shortName: String
    public let title: String
    public let count: Int
    public var isInstalled: Bool
    public let stickers: [FabushiStickerItem]

    public enum CodingKeys: String, CodingKey {
        case setId = "set_id"
        case shortName = "short_name"
        case title
        case count
        case isInstalled = "is_installed"
        case stickers
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.setId = try container.decodeIfPresent(String.self, forKey: .setId) ?? ""
        self.shortName = try container.decodeIfPresent(String.self, forKey: .shortName) ?? ""
        self.title = try container.decodeIfPresent(String.self, forKey: .title) ?? ""
        self.stickers = try container.decodeIfPresent([FabushiStickerItem].self, forKey: .stickers) ?? []
        self.count = try container.decodeIfPresent(Int.self, forKey: .count) ?? self.stickers.count
        self.isInstalled = try container.decodeIfPresent(Bool.self, forKey: .isInstalled) ?? false
    }
}

public struct FabushiGifItem: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: String
    public let title: String
    public let fileId: String
    public let url: String
    public let previewUrl: String
    public let width: UInt32
    public let height: UInt32
    public let mimeType: String

    public enum CodingKeys: String, CodingKey {
        case id
        case title
        case fileId = "file_id"
        case url
        case previewUrl = "preview_url"
        case width
        case height
        case mimeType = "mime_type"
    }
}

public struct FabushiMediaUploadResponse: Codable, Sendable, Equatable {
    public let fileId: String
    public let sha256: String?
    public let sizeBytes: UInt64
    public let mimeType: String
    public let filename: String?
    public let width: UInt32?
    public let height: UInt32?
    public let durationSeconds: UInt32?
    public let waveformB64: String?

    public enum CodingKeys: String, CodingKey {
        case fileId = "file_id"
        case sha256
        case sizeBytes = "size_bytes"
        case mimeType = "mime_type"
        case filename
        case width
        case height
        case durationSeconds = "duration_seconds"
        case waveformB64 = "waveform_b64"
    }
}

public struct FabushiMediaRangeResponse: Codable, Sendable, Equatable {
    public let status: Int
    public let sizeBytes: Int
    public let contentRange: String?
    public let dataB64: String

    public enum CodingKeys: String, CodingKey {
        case status
        case sizeBytes = "size_bytes"
        case contentRange = "content_range"
        case dataB64 = "data_b64"
    }
}

public enum FabushiInstantViewBlock: Codable, Sendable, Equatable, Hashable {
    case heading(String)
    case paragraph(String)
    case quote(String)
    case code(language: String?, content: String)
    case image(url: String, caption: String?)
    case divider
}

public struct FabushiInstantViewDocument: Identifiable, Codable, Sendable, Equatable, Hashable {
    public let id: String
    public let url: String
    public let siteName: String
    public let title: String
    public let author: String?
    public let publishedAt: String?
    public let blocks: [FabushiInstantViewBlock]

    public init(
        id: String = UUID().uuidString,
        url: String,
        siteName: String,
        title: String,
        author: String? = nil,
        publishedAt: String? = nil,
        blocks: [FabushiInstantViewBlock]
    ) {
        self.id = id
        self.url = url
        self.siteName = siteName
        self.title = title
        self.author = author
        self.publishedAt = publishedAt
        self.blocks = blocks
    }
}

public struct FabushiMediaViewerTarget: Identifiable, Sendable, Equatable, Hashable {
    public let id: String
    public let fileId: String
    public let fileName: String
    public let mimeType: String
    public let caption: String?
    public let isVideo: Bool
    public let chatId: Int64?

    public init(
        id: String = UUID().uuidString,
        fileId: String,
        fileName: String,
        mimeType: String,
        caption: String? = nil,
        isVideo: Bool = false,
        chatId: Int64? = nil
    ) {
        self.id = id
        self.fileId = fileId
        self.fileName = fileName
        self.mimeType = mimeType
        self.caption = caption
        self.isVideo = isVideo
        self.chatId = chatId
    }
}

// MARK: - Shared M3 Overlay Controller (Bridges M3a, M3b & Sealed `MessagingModuleProviding`)

@MainActor
@Observable
public final class FabushiMessagingOverlayController {
    public var isUniversalSearchPresented: Bool = false
    public var activeSearchChatId: Int64? = nil
    public var presentedMediaTarget: FabushiMediaViewerTarget? = nil
    public var presentedStoryIndex: Int? = nil
    public var isStoriesOverlayPresented: Bool = false
    public var presentedInstantView: FabushiInstantViewDocument? = nil
    public var presentedSafetyInfo: FabushiE2EESafetyInfo? = nil

    public init() {}

    public func openUniversalSearch(inChatId chatId: Int64? = nil) {
        activeSearchChatId = chatId
        isUniversalSearchPresented = true
    }

    public func dismissUniversalSearch() {
        isUniversalSearchPresented = false
        activeSearchChatId = nil
    }

    public func openMediaLightbox(_ target: FabushiMediaViewerTarget) {
        presentedMediaTarget = target
    }

    public func dismissMediaLightbox() {
        presentedMediaTarget = nil
    }

    public func openStories(startingAt index: Int = 0) {
        presentedStoryIndex = index
        isStoriesOverlayPresented = true
    }

    public func dismissStories() {
        isStoriesOverlayPresented = false
        presentedStoryIndex = nil
    }

    public func openInstantView(_ document: FabushiInstantViewDocument) {
        presentedInstantView = document
    }

    public func dismissInstantView() {
        presentedInstantView = nil
    }

    public func openE2EEVerification(_ info: FabushiE2EESafetyInfo) {
        presentedSafetyInfo = info
    }

    public func dismissE2EEVerification() {
        presentedSafetyInfo = nil
    }

    public func dismissAll() {
        dismissUniversalSearch()
        dismissMediaLightbox()
        dismissStories()
        dismissInstantView()
        dismissE2EEVerification()
    }
}
