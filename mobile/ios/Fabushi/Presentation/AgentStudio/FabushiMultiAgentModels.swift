import Foundation

// MARK: - Canonical Multi-Agent Protocol Constants (`D14-01` & `D14-02`)

public enum FabushiMultiAgentConstants: Sendable {
    /// Upstream `v1.6.14` group room floor lease window (`120_000 ms` = 120 seconds).
    public static let groupFloorLeaseMs: UInt64 = 120_000
    /// Default 5-hop loop budget cap (`gpui_telegram_agent::multi_agent::DEFAULT_MAX_LOOP_HOPS`).
    public static let defaultMaxLoopHops: UInt32 = 5
    /// Official default generalist bot user ID used in `GroupWakeReason::PrimaryFallback` priority.
    public static let officialDefaultBotUserId: Int64 = 777_000
}

// MARK: - Floor Control Queue (`multi_agent.floor*`)

public enum FabushiMultiAgentFloorPriority: String, CaseIterable, Codable, Sendable, Equatable {
    case userInterruption = "user_interruption"
    case directMention = "direct_mention"
    case taskAssignee = "task_assignee"
    case generalContribution = "general_contribution"
}

public struct FabushiMultiAgentFloorTurnGrantDTO: Codable, Sendable, Equatable {
    public let agentId: String
    public let nonce: String
    public let acquiredAtMs: Int64
    public let staleAfterMs: UInt64
    public let isWindingDown: Bool

    public enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case nonce
        case acquiredAtMs = "acquired_at_ms"
        case staleAfterMs = "stale_after_ms"
        case isWindingDown = "is_winding_down"
    }

    public init(
        agentId: String,
        nonce: String,
        acquiredAtMs: Int64,
        staleAfterMs: UInt64,
        isWindingDown: Bool
    ) {
        self.agentId = agentId
        self.nonce = nonce
        self.acquiredAtMs = acquiredAtMs
        self.staleAfterMs = staleAfterMs
        self.isWindingDown = isWindingDown
    }

    /// Computes non-negative remaining lease seconds at `nowMs` without false timeout flicker.
    public func remainingLeaseSeconds(atMs nowMs: Int64) -> Int {
        let leaseEndMs = acquiredAtMs &+ Int64(clamping: staleAfterMs)
        let remainingMs = max(0, leaseEndMs - nowMs)
        return Int((remainingMs + 999) / 1000)
    }
}

public struct FabushiMultiAgentFloorStatusDTO: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let granted: Bool?
    public let currentSpeaker: String?
    public let queuedCount: Int
    public let activeGrant: FabushiMultiAgentFloorTurnGrantDTO?

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case granted
        case currentSpeaker = "current_speaker"
        case queuedCount = "queued_count"
        case activeGrant = "active_grant"
    }

    public init(
        chatId: Int64,
        granted: Bool? = nil,
        currentSpeaker: String? = nil,
        queuedCount: Int = 0,
        activeGrant: FabushiMultiAgentFloorTurnGrantDTO? = nil
    ) {
        self.chatId = chatId
        self.granted = granted
        self.currentSpeaker = currentSpeaker
        self.queuedCount = queuedCount
        self.activeGrant = activeGrant
    }
}

public struct FabushiMultiAgentFloorPreemptDTO: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let preemptedAgent: String?
    public let preemptedNonce: String?
    public let currentSpeaker: String?
    public let queuedCount: Int

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case preemptedAgent = "preempted_agent"
        case preemptedNonce = "preempted_nonce"
        case currentSpeaker = "current_speaker"
        case queuedCount = "queued_count"
    }
}

public struct FabushiMultiAgentFloorReleaseDTO: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let currentSpeaker: String?
    public let queuedCount: Int

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case currentSpeaker = "current_speaker"
        case queuedCount = "queued_count"
    }
}

// MARK: - 5-Hop Loop Budget (`multi_agent.loopBudget*`)

public struct FabushiMultiAgentLoopBudgetStatusDTO: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let currentHop: UInt32
    public let maxHops: UInt32
    public let exhausted: Bool
    public let trace: [String]

    public var remainingHops: UInt32 {
        currentHop >= maxHops ? 0 : (maxHops - currentHop)
    }

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case currentHop = "current_hop"
        case maxHops = "max_hops"
        case exhausted
        case trace
    }

    public init(
        chatId: Int64,
        currentHop: UInt32 = 0,
        maxHops: UInt32 = FabushiMultiAgentConstants.defaultMaxLoopHops,
        exhausted: Bool = false,
        trace: [String] = []
    ) {
        self.chatId = chatId
        self.currentHop = currentHop
        self.maxHops = maxHops
        self.exhausted = exhausted
        self.trace = trace
    }
}

public struct FabushiMultiAgentLoopBudgetResetDTO: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let reset: Bool

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case reset
    }
}

// MARK: - Revision-Fenced `SharedGroupWorkspace` (`multi_agent.workspace*`)

public struct FabushiMultiAgentWorkspaceEntryDTO: Codable, Sendable, Equatable, Identifiable {
    public let key: String
    public let value: FabushiJSONValue
    public let updatedBy: String
    public let updatedAtMs: Int64

    public var id: String { key }

    public var displayValueText: String {
        if let text = value.stringValue {
            return text
        }
        return value.toJSONString()
    }

    public enum CodingKeys: String, CodingKey {
        case key
        case value
        case updatedBy = "updated_by"
        case updatedAtMs = "updated_at_ms"
    }
}

public struct FabushiMultiAgentWorkspaceSnapshotDTO: Codable, Sendable, Equatable {
    public let chatId: Int64
    public let revision: UInt64
    public let entries: [String: FabushiMultiAgentWorkspaceEntryDTO]

    public var sortedEntries: [FabushiMultiAgentWorkspaceEntryDTO] {
        entries.values.sorted { $0.key.localizedCaseInsensitiveCompare($1.key) == .orderedAscending }
    }

    public enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case revision
        case entries
    }

    public init(
        chatId: Int64,
        revision: UInt64 = 0,
        entries: [String: FabushiMultiAgentWorkspaceEntryDTO] = [:]
    ) {
        self.chatId = chatId
        self.revision = revision
        self.entries = entries
    }
}

public struct FabushiMultiAgentWorkspaceSetResponseDTO: Codable, Sendable, Equatable {
    public let revision: UInt64
    public let snapshot: FabushiMultiAgentWorkspaceSnapshotDTO
}

public struct FabushiMultiAgentWorkspaceRemoveResponseDTO: Codable, Sendable, Equatable {
    public let removed: FabushiMultiAgentWorkspaceEntryDTO?
}

public struct FabushiMultiAgentWorkspaceConflictState: Sendable, Equatable {
    public let key: String
    public let pendingValue: FabushiJSONValue
    public let expectedRevision: UInt64
    public let message: String

    public init(key: String, pendingValue: FabushiJSONValue, expectedRevision: UInt64, message: String) {
        self.key = key
        self.pendingValue = pendingValue
        self.expectedRevision = expectedRevision
        self.message = message
    }
}

// MARK: - Group Wake Gate & Stay-Silent `[PASS]` (`D14-01`, `multi_agent.evaluate*`, `isPassSentinel`)

public enum FabushiMultiAgentTurnOutcome: String, CaseIterable, Codable, Sendable, Equatable {
    case sent
    case pass
    case skipped
    case timeout
    case cancelled
    case error
}

/// All 9 canonical `GroupWakeReason` variants from `gpui_telegram_agent::multi_agent::reply_gate`.
public enum FabushiMultiAgentGroupWakeReason: String, CaseIterable, Codable, Sendable, Equatable {
    case explicitMention = "explicit_mention"
    case mentionAll = "mention_all"
    case replyToBot = "reply_to_bot"
    case singleBotRoom = "single_bot_room"
    case relevanceMatch = "relevance_match"
    case primaryFallback = "primary_fallback"
    case budgetExhausted = "budget_exhausted"
    case noRelevantCandidate = "no_relevant_candidate"
    case emptyCandidates = "empty_candidates"

    public var localizationKey: String {
        switch self {
        case .explicitMention: return "m4.multiAgent.wakeReason.explicitMention"
        case .mentionAll: return "m4.multiAgent.wakeReason.mentionAll"
        case .replyToBot: return "m4.multiAgent.wakeReason.replyToBot"
        case .singleBotRoom: return "m4.multiAgent.wakeReason.singleBotRoom"
        case .relevanceMatch: return "m4.multiAgent.wakeReason.relevanceMatch"
        case .primaryFallback: return "m4.multiAgent.wakeReason.primaryFallback"
        case .budgetExhausted: return "m4.multiAgent.wakeReason.budgetExhausted"
        case .noRelevantCandidate: return "m4.multiAgent.wakeReason.noRelevantCandidate"
        case .emptyCandidates: return "m4.multiAgent.wakeReason.emptyCandidates"
        }
    }
}

public struct FabushiMultiAgentWakeBotCandidateDTO: Codable, Sendable, Equatable, Identifiable {
    public let botUserId: Int64
    public let agentId: String
    public let name: String
    public let username: String
    public let role: String
    public let description: String
    public let skills: [String]
    public let secondsSinceLastReply: UInt64?

    public var id: Int64 { botUserId }

    public init(
        botUserId: Int64,
        agentId: String,
        name: String,
        username: String,
        role: String = "",
        description: String = "",
        skills: [String] = [],
        secondsSinceLastReply: UInt64? = nil
    ) {
        self.botUserId = botUserId
        self.agentId = agentId
        self.name = name
        self.username = username
        self.role = role
        self.description = description
        self.skills = skills
        self.secondsSinceLastReply = secondsSinceLastReply
    }

    public enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case agentId = "agent_id"
        case name
        case username
        case role
        case description
        case skills
        case secondsSinceLastReply = "seconds_since_last_reply"
    }
}

public struct FabushiMultiAgentWakeGateInputDTO: Codable, Sendable, Equatable {
    public let candidates: [FabushiMultiAgentWakeBotCandidateDTO]
    public let messageText: String
    public let explicitMentions: [String]
    public let replyToBotUserId: Int64?
    public let recentSpeakerBotUserId: Int64?
    public let senderIsHuman: Bool
    public let remainingLoopBudget: UInt32
    public let wakeThreshold: Float
    public let maxUnmentionedWakes: Int
    public let cooldownSeconds: UInt64

    public init(
        candidates: [FabushiMultiAgentWakeBotCandidateDTO],
        messageText: String,
        explicitMentions: [String] = [],
        replyToBotUserId: Int64? = nil,
        recentSpeakerBotUserId: Int64? = nil,
        senderIsHuman: Bool = true,
        remainingLoopBudget: UInt32 = FabushiMultiAgentConstants.defaultMaxLoopHops,
        wakeThreshold: Float = 0.35,
        maxUnmentionedWakes: Int = 1,
        cooldownSeconds: UInt64 = 15
    ) {
        self.candidates = candidates
        self.messageText = messageText
        self.explicitMentions = explicitMentions
        self.replyToBotUserId = replyToBotUserId
        self.recentSpeakerBotUserId = recentSpeakerBotUserId
        self.senderIsHuman = senderIsHuman
        self.remainingLoopBudget = remainingLoopBudget
        self.wakeThreshold = wakeThreshold
        self.maxUnmentionedWakes = maxUnmentionedWakes
        self.cooldownSeconds = cooldownSeconds
    }

    public enum CodingKeys: String, CodingKey {
        case candidates
        case messageText = "message_text"
        case explicitMentions = "explicit_mentions"
        case replyToBotUserId = "reply_to_bot_user_id"
        case recentSpeakerBotUserId = "recent_speaker_bot_user_id"
        case senderIsHuman = "sender_is_human"
        case remainingLoopBudget = "remaining_loop_budget"
        case wakeThreshold = "wake_threshold"
        case maxUnmentionedWakes = "max_unmentioned_wakes"
        case cooldownSeconds = "cooldown_seconds"
    }
}

public struct FabushiMultiAgentWakeBotScoreDTO: Codable, Sendable, Equatable, Identifiable {
    public let botUserId: Int64
    public let agentId: String
    public let score: Float
    public let reasons: [String]

    public var id: Int64 { botUserId }

    public enum CodingKeys: String, CodingKey {
        case botUserId = "bot_user_id"
        case agentId = "agent_id"
        case score
        case reasons
    }
}

public struct FabushiMultiAgentWakeGateDecisionDTO: Codable, Sendable, Equatable {
    public let shouldWake: Bool
    public let outcome: FabushiMultiAgentTurnOutcome
    public let wakeBotUserIds: [Int64]
    public let reason: FabushiMultiAgentGroupWakeReason
    public let scores: [FabushiMultiAgentWakeBotScoreDTO]
    public let isWindingDown: Bool

    public enum CodingKeys: String, CodingKey {
        case shouldWake = "should_wake"
        case outcome
        case wakeBotUserIds = "wake_bot_user_ids"
        case reason
        case scores
        case isWindingDown = "is_winding_down"
    }
}

public enum FabushiMultiAgentOutboundReviewReason: String, CaseIterable, Codable, Sendable, Equatable {
    case approved
    case emptyResponse = "empty_response"
    case explicitPassSentinel = "explicit_pass_sentinel"
    case duplicateEcho = "duplicate_echo"
}

public struct FabushiMultiAgentOutboundReviewInputDTO: Codable, Sendable, Equatable {
    public let agentId: String
    public let candidateText: String
    public let recentRoomMessages: [String]
    public let duplicateSimilarityThreshold: Float

    public init(
        agentId: String,
        candidateText: String,
        recentRoomMessages: [String] = [],
        duplicateSimilarityThreshold: Float = 0.85
    ) {
        self.agentId = agentId
        self.candidateText = candidateText
        self.recentRoomMessages = recentRoomMessages
        self.duplicateSimilarityThreshold = duplicateSimilarityThreshold
    }

    public enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case candidateText = "candidate_text"
        case recentRoomMessages = "recent_room_messages"
        case duplicateSimilarityThreshold = "duplicate_similarity_threshold"
    }
}

public struct FabushiMultiAgentOutboundVerdictDTO: Codable, Sendable, Equatable {
    public let outcome: FabushiMultiAgentTurnOutcome
    public let shouldSend: Bool
    public let reason: FabushiMultiAgentOutboundReviewReason
    public let sanitizedText: String
    public let maxDuplicateSimilarity: Float

    public enum CodingKeys: String, CodingKey {
        case outcome
        case shouldSend = "should_send"
        case reason
        case sanitizedText = "sanitized_text"
        case maxDuplicateSimilarity = "max_duplicate_similarity"
    }
}

public struct FabushiMultiAgentPassSentinelResultDTO: Codable, Sendable, Equatable {
    public let isPass: Bool

    public enum CodingKeys: String, CodingKey {
        case isPass = "is_pass"
    }
}

// MARK: - Scoped Ed25519 Capability Tokens (`multi_agent.issueCapabilityToken` / `verifyCapabilityToken`)

public struct FabushiMultiAgentCapabilityTokenDTO: Codable, Sendable, Equatable, Identifiable {
    public let tokenId: String
    public let agentId: String
    public let chatId: Int64
    public let allowedActions: [String]
    public let expiresAt: Int64
    public let signatureHex: String

    public var id: String { tokenId }

    public enum CodingKeys: String, CodingKey {
        case tokenId = "token_id"
        case agentId = "agent_id"
        case chatId = "chat_id"
        case allowedActions = "allowed_actions"
        case expiresAt = "expires_at"
        case signatureHex = "signature_hex"
    }
}

public struct FabushiMultiAgentIssuedCapabilityTokenDTO: Codable, Sendable, Equatable {
    public let token: FabushiMultiAgentCapabilityTokenDTO
    public let verifyingKeyHex: String

    public enum CodingKeys: String, CodingKey {
        case token
        case verifyingKeyHex = "verifying_key_hex"
    }
}

public struct FabushiMultiAgentVerifyCapabilityResultDTO: Codable, Sendable, Equatable {
    public let valid: Bool
    public let permitsAction: Bool?

    public enum CodingKeys: String, CodingKey {
        case valid
        case permitsAction = "permits_action"
    }
}

// MARK: - Live Handoff Telemetry

public struct FabushiMultiAgentHandoffRecord: Identifiable, Sendable, Equatable {
    public let id: UUID
    public let chatId: Int64
    public let fromAgent: String
    public let toAgent: String
    public let hop: Int
    public let maxHops: Int
    public let timestampMs: Int64

    public init(
        id: UUID = UUID(),
        chatId: Int64,
        fromAgent: String,
        toAgent: String,
        hop: Int,
        maxHops: Int,
        timestampMs: Int64
    ) {
        self.id = id
        self.chatId = chatId
        self.fromAgent = fromAgent
        self.toAgent = toAgent
        self.hop = hop
        self.maxHops = maxHops
        self.timestampMs = timestampMs
    }
}
