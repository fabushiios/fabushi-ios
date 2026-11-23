import Foundation
import os

// MARK: - Strongly Typed Telegram Events

public enum TelegramEvent: Sendable, Equatable {
    case newMessage(id: Int64, chatId: Int64, senderId: Int64, text: String, timestamp: Int64, replyTo: Int64? = nil)
    case editMessage(id: Int64, chatId: Int64, text: String, editTimestamp: Int64)
    case deleteMessages(chatId: Int64, messageIds: [Int64])
    case ptsUpdate(pts: UInt64, ptsCount: UInt32)
    case syncDifference(pts: UInt64, hasMore: Bool)
    case userTyping(chatId: Int64, userId: Int64, action: String)
    case readHistory(chatId: Int64, maxId: Int64)
    case reactionUpdated(chatId: Int64, messageId: Int64, emoji: String, count: Int, chosenByMe: Bool)
    case userStatusChanged(userId: Int64, isOnline: Bool)
    case connectionStateChanged(state: String)
    case taskStateUpdated(taskId: String, state: String)
    case toolApprovalRequested(approvalId: String, toolName: String, summary: String)
    case raw(type: String, json: String)
}

// MARK: - Bounded Multi-Subscriber Event Stream Pipeline

public final class TelegramEventStream: Sendable {
    public static let defaultBufferCapacity = 1024

    private let maxBufferedEvents: Int
    private let lock = OSAllocatedUnfairLock<[UUID: AsyncStream<TelegramEvent>.Continuation]>(
        initialState: [:]
    )

    public init(maxBufferedEvents: Int = defaultBufferCapacity) {
        self.maxBufferedEvents = max(1, maxBufferedEvents)
    }

    public var stream: AsyncStream<TelegramEvent> {
        makeStream()
    }

    public func makeStream(bufferCapacity: Int? = nil) -> AsyncStream<TelegramEvent> {
        let id = UUID()
        let capacity = max(1, bufferCapacity ?? maxBufferedEvents)
        return AsyncStream(bufferingPolicy: .bufferingNewest(capacity)) { continuation in
            lock.withLock { map in
                map[id] = continuation
            }

            continuation.onTermination = { [weak self] _ in
                self?.lock.withLock { map in
                    _ = map.removeValue(forKey: id)
                }
            }
        }
    }

    public func yield(_ event: TelegramEvent) {
        let targets = lock.withLock { Array($0.values) }
        for continuation in targets {
            continuation.yield(event)
        }
    }

    public func yield(domainEvent: FabushiDomainEvent) {
        yield(domainEvent.toTelegramEvent())
    }

    public func finish() {
        let targets = lock.withLock { map -> [AsyncStream<TelegramEvent>.Continuation] in
            let values = Array(map.values)
            map.removeAll()
            return values
        }
        for continuation in targets {
            continuation.finish()
        }
    }

    public func parseAndYield(type: String, json: String) {
        let event = Self.parseEvent(type: type, json: json)
        yield(event)
    }

    /// Parses raw event JSON through the canonical M1b `FabushiDomainEvent` schema
    /// (including `"status"` for `connectionStateChanged`, `"reactions"` array for
    /// `reactionUpdated`, and `"status"` for `userStatusChanged`) and bridges into `TelegramEvent`.
    public static func parseEvent(type: String, json: String) -> TelegramEvent {
        FabushiDomainEvent.parse(type: type, rawPayloadJSON: json).toTelegramEvent()
    }
}
