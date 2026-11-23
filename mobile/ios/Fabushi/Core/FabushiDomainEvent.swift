import Foundation

// MARK: - Canonical Domain Event Reaction Summary

public struct FabushiReactionSummary: Sendable, Equatable {
    public let emoji: String
    public let count: Int
    public let chosenByMe: Bool

    public init(emoji: String, count: Int, chosenByMe: Bool) {
        self.emoji = emoji
        self.count = count
        self.chosenByMe = chosenByMe
    }
}

// MARK: - Canonical Fabushi Domain Event (Exhaustive M1b source/host/src/events.rs Schema)

public struct FabushiDomainEvent: Sendable, Equatable {
    public enum Kind: Sendable, Equatable {
        case newMessage(
            id: Int64,
            chatId: Int64,
            senderId: Int64,
            text: String,
            timestamp: Int64,
            replyTo: Int64?,
            clientNonce: UInt64?,
            status: String?
        )
        case editMessage(id: Int64, chatId: Int64, text: String, editTimestamp: Int64)
        case deleteMessages(chatId: Int64, messageIds: [Int64], forEveryone: Bool)
        case reactionUpdated(
            chatId: Int64,
            messageId: Int64,
            userId: Int64,
            reactions: [FabushiReactionSummary]
        )
        case pollUpdated(chatId: Int64, messageId: Int64, payload: FabushiJSONValue)
        case todoUpdated(chatId: Int64, messageId: Int64, payload: FabushiJSONValue)
        case readHistory(chatId: Int64, maxId: Int64, isOutbox: Bool, unreadCount: Int)
        case userTyping(chatId: Int64, userId: Int64, action: String)
        case userStatusChanged(userId: Int64, status: String, isOnline: Bool)
        case pinnedMessagesChanged(chatId: Int64, pinnedIds: [Int64])
        case draftUpdated(chatId: Int64, topicId: Int64?, payload: FabushiJSONValue)
        case forumTopicChanged(chatId: Int64, payload: FabushiJSONValue)
        case folderChanged(payload: FabushiJSONValue)
        case callUpdated(callId: String, state: String, payload: FabushiJSONValue)
        case storyUpdated(payload: FabushiJSONValue)
        case agentFloorChanged(chatId: Int64, currentSpeaker: String?, queuedCount: Int, loopHops: Int)
        case agentWorkspaceUpdated(chatId: Int64, revision: UInt64, key: String, updatedBy: String?)
        case agentHandoffEvent(chatId: Int64, fromAgent: String, toAgent: String, hop: Int, maxHops: Int)
        case botActivityChanged(chatId: Int64, botUserId: Int64, state: String, detail: String?)
        case miniAppWebMcpCommand(payload: FabushiJSONValue)
        case miniAppControlStatus(
            botUsername: String,
            shortName: String,
            chatId: Int64,
            botUserId: Int64,
            appTitle: String,
            mode: String,
            statusText: String?,
            activeTool: String?
        )
        case codingHostCommand(payload: FabushiJSONValue)
        case chatAction(chatId: Int64?, action: String)
        case messageAck(correlationId: String?, clientNonce: UInt64?, messageId: Int64, pts: UInt64?)
        case syncGapDetected(chatId: Int64, fromPts: UInt64)
        case serverError(code: String, message: String, details: FabushiJSONValue?)
        case pong(nonce: UInt64)
        case outboxStatusChanged(
            clientNonce: UInt64,
            chatId: Int64,
            status: String,
            serverMessageId: Int64?,
            retryCount: Int,
            lastError: String?
        )
        case connectionStateChanged(status: String, serverUrl: String?)
        case taskStateUpdated(taskId: String, state: String)
        case syncDifference(chatId: Int64?, pts: UInt64, hasMore: Bool, updatesCount: Int)
        case remoteEmergencyTakeover(disconnectedSessions: Int, overrideEpoch: UInt64, elapsedUs: UInt64)
        case toolApprovalRequested(approvalId: String, toolName: String, summary: String)
        case ptsUpdate(pts: UInt64, ptsCount: UInt32)
        case raw(type: String, json: String)
    }

    public let seq: UInt64
    public let generation: UInt64
    public let timestampMs: Int64
    public let type: String
    public let rawPayloadJSON: String
    public let kind: Kind

    public init(
        seq: UInt64,
        generation: UInt64,
        timestampMs: Int64,
        type: String,
        rawPayloadJSON: String,
        kind: Kind
    ) {
        self.seq = seq
        self.generation = generation
        self.timestampMs = timestampMs
        self.type = type
        self.rawPayloadJSON = rawPayloadJSON
        self.kind = kind
    }

    // MARK: - Wire Envelope Decoding (fabushi_host_poll_events)

    struct WireHostEvent: Decodable, Sendable {
        let seq: UInt64
        let generation: UInt64
        let timestampMs: Int64
        let type: String
        let payload: FabushiJSONValue

        enum CodingKeys: String, CodingKey {
            case seq
            case generation
            case timestampMs = "timestamp_ms"
            case type
            case payload
        }
    }

    static func fromWire(_ wire: WireHostEvent) -> FabushiDomainEvent {
        let rawJSON = wire.payload.toJSONString()
        let kind = parseKind(type: wire.type, payload: wire.payload, rawJSON: rawJSON)
        return FabushiDomainEvent(
            seq: wire.seq,
            generation: wire.generation,
            timestampMs: wire.timestampMs,
            type: wire.type,
            rawPayloadJSON: rawJSON,
            kind: kind
        )
    }

    public static func parse(
        seq: UInt64 = 0,
        generation: UInt64 = 0,
        timestampMs: Int64 = 0,
        type: String,
        rawPayloadJSON: String
    ) -> FabushiDomainEvent {
        let payload: FabushiJSONValue
        if let data = rawPayloadJSON.data(using: .utf8),
           let decoded = try? JSONDecoder().decode(FabushiJSONValue.self, from: data)
        {
            payload = decoded
        } else {
            payload = .null
        }
        let kind = parseKind(type: type, payload: payload, rawJSON: rawPayloadJSON)
        return FabushiDomainEvent(
            seq: seq,
            generation: generation,
            timestampMs: timestampMs,
            type: type,
            rawPayloadJSON: rawPayloadJSON,
            kind: kind
        )
    }

    // MARK: - Typed Kind Parser

    private static func parseKind(type: String, payload: FabushiJSONValue, rawJSON: String) -> Kind {
        guard case .object(let dict) = payload else {
            return .raw(type: type, json: rawJSON)
        }

        switch type {
        case "newMessage":
            let id = dict["id"]?.int64Value ?? dict["message_id"]?.int64Value ?? 0
            let chatId = dict["chat_id"]?.int64Value ?? 0
            let senderId = dict["sender_id"]?.int64Value ?? 0
            let text = dict["text"]?.stringValue ?? dict["body"]?.stringValue ?? ""
            let timestamp = dict["timestamp"]?.int64Value ?? dict["date"]?.int64Value ?? 0
            let replyTo = dict["reply_to"]?.int64Value ?? dict["reply_to_message_id"]?.int64Value
            let clientNonce = dict["client_nonce"]?.uint64Value
            let status = dict["status"]?.stringValue
            return .newMessage(
                id: id,
                chatId: chatId,
                senderId: senderId,
                text: text,
                timestamp: timestamp,
                replyTo: replyTo,
                clientNonce: clientNonce,
                status: status
            )

        case "editMessage":
            let id = dict["id"]?.int64Value ?? dict["message_id"]?.int64Value ?? 0
            let chatId = dict["chat_id"]?.int64Value ?? 0
            let text = dict["text"]?.stringValue ?? dict["body"]?.stringValue ?? ""
            let editTs = dict["edit_timestamp"]?.int64Value ?? dict["edit_date"]?.int64Value ?? dict["timestamp"]?.int64Value ?? 0
            return .editMessage(id: id, chatId: chatId, text: text, editTimestamp: editTs)

        case "deleteMessages":
            let chatId = dict["chat_id"]?.int64Value ?? 0
            let ids = dict["message_ids"]?.arrayValue?.compactMap(\.int64Value) ?? []
            let forEveryone = dict["for_everyone"]?.boolValue ?? false
            return .deleteMessages(chatId: chatId, messageIds: ids, forEveryone: forEveryone)

        case "reactionUpdated":
            let chatId = dict["chat_id"]?.int64Value ?? 0
            let messageId = dict["message_id"]?.int64Value ?? 0
            let userId = dict["user_id"]?.int64Value ?? 0
            var summaries: [FabushiReactionSummary] = []
            if let list = dict["reactions"]?.arrayValue {
                for item in list {
                    if let obj = item.objectValue {
                        let emoji = obj["emoji"]?.stringValue ?? obj["reaction"]?.stringValue ?? ""
                        let count = Int(obj["count"]?.int64Value ?? 1)
                        let mine = obj["chosen_by_me"]?.boolValue ?? obj["me"]?.boolValue ?? obj["chosen"]?.boolValue ?? false
                        if !emoji.isEmpty {
                            summaries.append(FabushiReactionSummary(emoji: emoji, count: count, chosenByMe: mine))
                        }
                    } else if let emoji = item.stringValue, !emoji.isEmpty {
                        summaries.append(FabushiReactionSummary(emoji: emoji, count: 1, chosenByMe: false))
                    }
                }
            } else if let emoji = dict["emoji"]?.stringValue, !emoji.isEmpty {
                let count = Int(dict["count"]?.int64Value ?? 0)
                let mine = dict["chosen_by_me"]?.boolValue ?? false
                summaries.append(FabushiReactionSummary(emoji: emoji, count: count, chosenByMe: mine))
            }
            return .reactionUpdated(chatId: chatId, messageId: messageId, userId: userId, reactions: summaries)

        case "pollUpdated":
            return .pollUpdated(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                messageId: dict["message_id"]?.int64Value ?? 0,
                payload: dict["poll"] ?? payload
            )

        case "todoUpdated":
            return .todoUpdated(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                messageId: dict["message_id"]?.int64Value ?? 0,
                payload: dict["todo"] ?? payload
            )

        case "readHistory":
            return .readHistory(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                maxId: dict["max_id"]?.int64Value ?? 0,
                isOutbox: dict["is_outbox"]?.boolValue ?? false,
                unreadCount: Int(dict["unread_count"]?.int64Value ?? 0)
            )

        case "userTyping":
            let chatId = dict["chat_id"]?.int64Value ?? 0
            let userId = dict["user_id"]?.int64Value ?? 0
            let action = dict["action"]?.stringValue ?? "typing"
            return .userTyping(chatId: chatId, userId: userId, action: action)

        case "userStatusChanged":
            let userId = dict["user_id"]?.int64Value ?? 0
            let (statusText, isOnline) = parseUserStatus(dict: dict)
            return .userStatusChanged(userId: userId, status: statusText, isOnline: isOnline)

        case "pinnedMessagesChanged":
            let chatId = dict["chat_id"]?.int64Value ?? 0
            let pinnedIds = dict["pinned_ids"]?.arrayValue?.compactMap(\.int64Value) ?? []
            return .pinnedMessagesChanged(chatId: chatId, pinnedIds: pinnedIds)

        case "draftUpdated":
            return .draftUpdated(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                topicId: dict["topic_id"]?.int64Value,
                payload: dict["draft"] ?? payload
            )

        case "forumTopicChanged":
            return .forumTopicChanged(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                payload: dict["topic"] ?? payload
            )

        case "folderChanged":
            return .folderChanged(payload: dict["folder"] ?? payload)

        case "callUpdated":
            let callId = dict["call_id"]?.stringValue ?? ""
            let state = dict["state"]?.stringValue ?? dict["state"]?.toJSONString() ?? "unknown"
            return .callUpdated(callId: callId, state: state, payload: payload)

        case "storyUpdated":
            return .storyUpdated(payload: dict["story"] ?? payload)

        case "agentFloorChanged":
            return .agentFloorChanged(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                currentSpeaker: dict["current_speaker"]?.stringValue,
                queuedCount: Int(dict["queued_count"]?.int64Value ?? 0),
                loopHops: Int(dict["loop_hops"]?.int64Value ?? 0)
            )

        case "agentWorkspaceUpdated":
            return .agentWorkspaceUpdated(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                revision: dict["revision"]?.uint64Value ?? 0,
                key: dict["key"]?.stringValue ?? "",
                updatedBy: dict["updated_by"]?.stringValue
            )

        case "agentHandoffEvent":
            return .agentHandoffEvent(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                fromAgent: dict["from_agent"]?.stringValue ?? "",
                toAgent: dict["to_agent"]?.stringValue ?? "",
                hop: Int(dict["hop"]?.int64Value ?? 0),
                maxHops: Int(dict["max_hops"]?.int64Value ?? 5)
            )

        case "botActivityChanged":
            return .botActivityChanged(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                botUserId: dict["bot_user_id"]?.int64Value ?? 0,
                state: dict["state"]?.stringValue ?? "",
                detail: dict["detail"]?.stringValue
            )

        case "miniAppWebMcpCommand":
            return .miniAppWebMcpCommand(payload: payload)

        case "miniAppControlStatus":
            return .miniAppControlStatus(
                botUsername: dict["bot_username"]?.stringValue ?? "",
                shortName: dict["short_name"]?.stringValue ?? "",
                chatId: dict["chat_id"]?.int64Value ?? 0,
                botUserId: dict["bot_user_id"]?.int64Value ?? 0,
                appTitle: dict["app_title"]?.stringValue ?? "",
                mode: dict["mode"]?.stringValue ?? "",
                statusText: dict["status_text"]?.stringValue,
                activeTool: dict["active_tool"]?.stringValue
            )

        case "codingHostCommand":
            return .codingHostCommand(payload: payload)

        case "chatAction":
            let chatId = dict["chat_id"]?.int64Value
            let action = dict["action"]?.stringValue ?? dict["action"]?.toJSONString() ?? ""
            return .chatAction(chatId: chatId, action: action)

        case "messageAck":
            return .messageAck(
                correlationId: dict["correlation_id"]?.stringValue,
                clientNonce: dict["client_nonce"]?.uint64Value,
                messageId: dict["message_id"]?.int64Value ?? 0,
                pts: dict["pts"]?.uint64Value
            )

        case "syncGapDetected":
            return .syncGapDetected(
                chatId: dict["chat_id"]?.int64Value ?? 0,
                fromPts: dict["from_pts"]?.uint64Value ?? 0
            )

        case "serverError":
            return .serverError(
                code: dict["code"]?.stringValue ?? "server_error",
                message: dict["message"]?.stringValue ?? "",
                details: dict["details"]
            )

        case "pong":
            return .pong(nonce: dict["nonce"]?.uint64Value ?? 0)

        case "outboxStatusChanged":
            return .outboxStatusChanged(
                clientNonce: dict["client_nonce"]?.uint64Value ?? 0,
                chatId: dict["chat_id"]?.int64Value ?? 0,
                status: dict["status"]?.stringValue ?? "unknown",
                serverMessageId: dict["server_message_id"]?.int64Value,
                retryCount: Int(dict["retry_count"]?.int64Value ?? 0),
                lastError: dict["last_error"]?.stringValue
            )

        case "connectionStateChanged":
            let status = dict["status"]?.stringValue ?? dict["state"]?.stringValue ?? "unknown"
            let serverUrl = dict["server_url"]?.stringValue
            return .connectionStateChanged(status: status, serverUrl: serverUrl)

        case "taskStateUpdated":
            let taskId = dict["taskId"]?.stringValue ?? dict["task_id"]?.stringValue ?? ""
            let state = dict["state"]?.stringValue ?? "pending"
            return .taskStateUpdated(taskId: taskId, state: state)

        case "syncDifference":
            return .syncDifference(
                chatId: dict["chat_id"]?.int64Value,
                pts: dict["pts"]?.uint64Value ?? 0,
                hasMore: dict["has_more"]?.boolValue ?? false,
                updatesCount: Int(dict["updates_count"]?.int64Value ?? 0)
            )

        case "remoteEmergencyTakeover":
            return .remoteEmergencyTakeover(
                disconnectedSessions: Int(dict["disconnected_sessions"]?.int64Value ?? 0),
                overrideEpoch: dict["override_epoch"]?.uint64Value ?? 0,
                elapsedUs: dict["elapsed_us"]?.uint64Value ?? 0
            )

        case "toolApprovalRequested":
            return .toolApprovalRequested(
                approvalId: dict["approvalId"]?.stringValue ?? dict["approval_id"]?.stringValue ?? "",
                toolName: dict["toolName"]?.stringValue ?? dict["tool_name"]?.stringValue ?? "",
                summary: dict["summary"]?.stringValue ?? ""
            )

        case "ptsUpdate":
            return .ptsUpdate(
                pts: dict["pts"]?.uint64Value ?? 0,
                ptsCount: UInt32(dict["pts_count"]?.uint64Value ?? 0)
            )

        default:
            return .raw(type: type, json: rawJSON)
        }
    }

    private static func parseUserStatus(dict: [String: FabushiJSONValue]) -> (String, Bool) {
        if let statusValue = dict["status"] {
            switch statusValue {
            case .string(let s):
                let lower = s.lowercased()
                return (s, lower == "online")
            case .object(let obj):
                if let tag = obj["type"]?.stringValue ?? obj["kind"]?.stringValue {
                    return (tag, tag.lowercased() == "online")
                }
                if obj["Online"] != nil || obj["online"] != nil {
                    return ("online", true)
                }
                if let firstKey = obj.keys.sorted().first {
                    return (firstKey, firstKey.lowercased() == "online")
                }
            default:
                break
            }
        }
        if let legacyOnline = dict["is_online"]?.boolValue {
            return (legacyOnline ? "online" : "offline", legacyOnline)
        }
        return ("unknown", false)
    }

    // MARK: - Lossless Bridge to Legacy Presentation TelegramEvent

    public func toTelegramEvent() -> TelegramEvent {
        switch kind {
        case .newMessage(let id, let chatId, let senderId, let text, let timestamp, let replyTo, _, _):
            return .newMessage(
                id: id,
                chatId: chatId,
                senderId: senderId,
                text: text,
                timestamp: timestamp,
                replyTo: replyTo
            )
        case .editMessage(let id, let chatId, let text, let editTimestamp):
            return .editMessage(id: id, chatId: chatId, text: text, editTimestamp: editTimestamp)
        case .deleteMessages(let chatId, let messageIds, _):
            return .deleteMessages(chatId: chatId, messageIds: messageIds)
        case .ptsUpdate(let pts, let ptsCount):
            return .ptsUpdate(pts: pts, ptsCount: ptsCount)
        case .syncDifference(_, let pts, let hasMore, _):
            return .syncDifference(pts: pts, hasMore: hasMore)
        case .userTyping(let chatId, let userId, let action):
            return .userTyping(chatId: chatId, userId: userId, action: action)
        case .readHistory(let chatId, let maxId, _, _):
            return .readHistory(chatId: chatId, maxId: maxId)
        case .reactionUpdated(let chatId, let messageId, _, let reactions):
            let primary = reactions.first
            return .reactionUpdated(
                chatId: chatId,
                messageId: messageId,
                emoji: primary?.emoji ?? "",
                count: primary?.count ?? 0,
                chosenByMe: primary?.chosenByMe ?? false
            )
        case .userStatusChanged(let userId, _, let isOnline):
            return .userStatusChanged(userId: userId, isOnline: isOnline)
        case .connectionStateChanged(let status, _):
            return .connectionStateChanged(state: status)
        case .taskStateUpdated(let taskId, let state):
            return .taskStateUpdated(taskId: taskId, state: state)
        case .toolApprovalRequested(let approvalId, let toolName, let summary):
            return .toolApprovalRequested(approvalId: approvalId, toolName: toolName, summary: summary)
        default:
            return .raw(type: type, json: rawPayloadJSON)
        }
    }
}
