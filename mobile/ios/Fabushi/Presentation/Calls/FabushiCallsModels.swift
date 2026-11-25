import Foundation

// MARK: - Call Session State & DTOs (Feature #29)

public enum FabushiCallSessionState: String, Codable, Sendable, Equatable {
    case idle
    case ringingOutgoing = "ringing_outgoing"
    case ringingIncoming = "ringing_incoming"
    case connecting
    case connected
    case reconnecting
    case ended
    case failed

    public var isActive: Bool {
        switch self {
        case .ringingOutgoing, .ringingIncoming, .connecting, .connected, .reconnecting:
            return true
        case .idle, .ended, .failed:
            return false
        }
    }

    public func localizedTitle(locale: FabushiLocale) -> String {
        switch self {
        case .idle:
            return FabushiPlatformStrings.text(.callsStateIdle, locale: locale)
        case .ringingOutgoing:
            return FabushiPlatformStrings.text(.callsStateRingingOutgoing, locale: locale)
        case .ringingIncoming:
            return FabushiPlatformStrings.text(.callsStateRingingIncoming, locale: locale)
        case .connecting:
            return FabushiPlatformStrings.text(.callsStateConnecting, locale: locale)
        case .connected:
            return FabushiPlatformStrings.text(.callsStateConnected, locale: locale)
        case .reconnecting:
            return FabushiPlatformStrings.text(.callsStateReconnecting, locale: locale)
        case .ended:
            return FabushiPlatformStrings.text(.callsStateEnded, locale: locale)
        case .failed:
            return FabushiPlatformStrings.text(.callsStateFailed, locale: locale)
        }
    }
}

public struct FabushiCallRecordDto: Codable, Identifiable, Sendable, Equatable {
    public let callId: String
    public let callerId: UInt64
    public let calleeId: UInt64
    public let isVideo: Bool
    public let state: String
    public let durationSecs: UInt32
    public let rating: UInt8?
    public let startedAtMs: Int64
    public let endedAtMs: Int64?

    public var id: String { callId }

    enum CodingKeys: String, CodingKey {
        case callId = "call_id"
        case callerId = "caller_id"
        case calleeId = "callee_id"
        case isVideo = "is_video"
        case state
        case durationSecs = "duration_secs"
        case rating
        case startedAtMs = "started_at_ms"
        case endedAtMs = "ended_at_ms"
    }

    public init(
        callId: String,
        callerId: UInt64,
        calleeId: UInt64,
        isVideo: Bool = false,
        state: String = "ended",
        durationSecs: UInt32 = 0,
        rating: UInt8? = nil,
        startedAtMs: Int64 = 0,
        endedAtMs: Int64? = nil
    ) {
        self.callId = callId
        self.callerId = callerId
        self.calleeId = calleeId
        self.isVideo = isVideo
        self.state = state
        self.durationSecs = durationSecs
        self.rating = rating
        self.startedAtMs = startedAtMs
        self.endedAtMs = endedAtMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        callId = try c.decode(String.self, forKey: .callId)
        callerId = try c.decodeIfPresent(UInt64.self, forKey: .callerId) ?? 0
        calleeId = try c.decodeIfPresent(UInt64.self, forKey: .calleeId) ?? 0
        isVideo = try c.decodeIfPresent(Bool.self, forKey: .isVideo) ?? false
        state = try c.decodeIfPresent(String.self, forKey: .state) ?? "ended"
        durationSecs = try c.decodeIfPresent(UInt32.self, forKey: .durationSecs) ?? 0
        rating = try c.decodeIfPresent(UInt8.self, forKey: .rating)
        startedAtMs = try c.decodeIfPresent(Int64.self, forKey: .startedAtMs) ?? 0
        endedAtMs = try c.decodeIfPresent(Int64.self, forKey: .endedAtMs)
    }
}

public struct FabushiCallInviteRequest: Encodable, Sendable {
    public let calleeId: UInt64
    public let isVideo: Bool
    public let chatId: Int64?

    enum CodingKeys: String, CodingKey {
        case calleeId = "callee_id"
        case isVideo = "is_video"
        case chatId = "chat_id"
    }

    public init(calleeId: UInt64, isVideo: Bool = false, chatId: Int64? = nil) {
        self.calleeId = calleeId
        self.isVideo = isVideo
        self.chatId = chatId
    }
}

public struct FabushiCallIdParams: Encodable, Sendable {
    public let callId: String

    enum CodingKeys: String, CodingKey {
        case callId = "call_id"
    }

    public init(callId: String) {
        self.callId = callId
    }
}

public struct FabushiCallRateRequest: Encodable, Sendable {
    public let callId: String
    public let rating: UInt8
    public let feedback: String?

    enum CodingKeys: String, CodingKey {
        case callId = "call_id"
        case rating
        case feedback
    }

    public init(callId: String, rating: UInt8, feedback: String? = nil) {
        self.callId = callId
        self.rating = min(5, max(1, rating))
        let trimmed = feedback?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.feedback = (trimmed?.isEmpty == false) ? trimmed : nil
    }
}

public struct FabushiCallSignalRequest: Encodable, Sendable {
    public let callId: String
    public let signalType: String
    public let sdp: String?
    public let candidate: String?
    public let sdpMid: String?
    public let sdpMlineIndex: UInt32?
    public let reason: String?
    public let targetUserId: UInt64?

    enum CodingKeys: String, CodingKey {
        case callId = "call_id"
        case signalType = "signal_type"
        case sdp
        case candidate
        case sdpMid = "sdp_mid"
        case sdpMlineIndex = "sdp_mline_index"
        case reason
        case targetUserId = "target_user_id"
    }

    public init(
        callId: String,
        signalType: String,
        sdp: String? = nil,
        candidate: String? = nil,
        sdpMid: String? = nil,
        sdpMlineIndex: UInt32? = nil,
        reason: String? = nil,
        targetUserId: UInt64? = nil
    ) {
        self.callId = callId
        self.signalType = signalType
        self.sdp = sdp
        self.candidate = candidate
        self.sdpMid = sdpMid
        self.sdpMlineIndex = sdpMlineIndex
        self.reason = reason
        self.targetUserId = targetUserId
    }
}

public struct FabushiCallEngineStartResult: Decodable, Sendable, Equatable {
    public let callId: Int64
    public let localSdp: String?
    public let outboundFrames: [FabushiJSONValue]

    enum CodingKeys: String, CodingKey {
        case callId = "call_id"
        case localSdp = "local_sdp"
        case outboundFrames = "outbound_frames"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        callId = try c.decodeIfPresent(Int64.self, forKey: .callId) ?? 0
        localSdp = try c.decodeIfPresent(String.self, forKey: .localSdp)
        outboundFrames = try c.decodeIfPresent([FabushiJSONValue].self, forKey: .outboundFrames) ?? []
    }
}

public struct FabushiCallEngineAcceptResult: Decodable, Sendable, Equatable {
    public let answer: String?
    public let outboundFrames: [FabushiJSONValue]
    public let emojiFingerprint: String?

    enum CodingKeys: String, CodingKey {
        case answer
        case outboundFrames = "outbound_frames"
        case emojiFingerprint = "emoji_fingerprint"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        answer = try c.decodeIfPresent(String.self, forKey: .answer)
        outboundFrames = try c.decodeIfPresent([FabushiJSONValue].self, forKey: .outboundFrames) ?? []
        emojiFingerprint = try c.decodeIfPresent(String.self, forKey: .emojiFingerprint)
    }
}

public struct FabushiCallEngineApplySignalResult: Decodable, Sendable, Equatable {
    public let applied: Bool
    public let emojiFingerprint: String?
    public let outboundFrames: [FabushiJSONValue]

    enum CodingKeys: String, CodingKey {
        case applied
        case emojiFingerprint = "emoji_fingerprint"
        case outboundFrames = "outbound_frames"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        applied = try c.decodeIfPresent(Bool.self, forKey: .applied) ?? false
        emojiFingerprint = try c.decodeIfPresent(String.self, forKey: .emojiFingerprint)
        outboundFrames = try c.decodeIfPresent([FabushiJSONValue].self, forKey: .outboundFrames) ?? []
    }
}

public struct FabushiCallEmojiFingerprintResult: Decodable, Sendable, Equatable {
    public let emojis: String?
}

public struct FabushiCallAudioLevelResult: Decodable, Sendable, Equatable {
    public let level: Double
    public let isSpeaking: Bool

    enum CodingKeys: String, CodingKey {
        case level
        case isSpeaking = "is_speaking"
    }
}

public struct FabushiCallSfuRoomResult: Decodable, Sendable, Equatable {
    public let chatId: Int64
    public let subscribers: [UInt64]

    enum CodingKeys: String, CodingKey {
        case chatId = "chat_id"
        case subscribers
    }
}

// MARK: - Full-Duplex AI Talk Mode DTOs (Feature #29)

public enum FabushiTalkSessionState: String, Codable, Sendable, Equatable {
    case idle
    case listening
    case thinking
    case speaking
    case interrupted
    case reconnecting
    case closed

    public var isRunning: Bool {
        switch self {
        case .listening, .thinking, .speaking, .interrupted, .reconnecting:
            return true
        case .idle, .closed:
            return false
        }
    }

    public func localizedTitle(locale: FabushiLocale) -> String {
        switch self {
        case .idle:
            return FabushiPlatformStrings.text(.talkModeStateIdle, locale: locale)
        case .listening:
            return FabushiPlatformStrings.text(.talkModeStateListening, locale: locale)
        case .thinking:
            return FabushiPlatformStrings.text(.talkModeStateThinking, locale: locale)
        case .speaking:
            return FabushiPlatformStrings.text(.talkModeStateSpeaking, locale: locale)
        case .interrupted:
            return FabushiPlatformStrings.text(.talkModeStateInterrupted, locale: locale)
        case .reconnecting:
            return FabushiPlatformStrings.text(.talkModeStateReconnecting, locale: locale)
        case .closed:
            return FabushiPlatformStrings.text(.talkModeStateClosed, locale: locale)
        }
    }
}

public struct FabushiTalkSessionCreateResult: Decodable, Sendable, Equatable {
    public let sessionId: String
    public let state: String
    public let generation: UInt64

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case state
        case generation
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try c.decodeIfPresent(String.self, forKey: .sessionId) ?? ""
        state = try c.decodeIfPresent(String.self, forKey: .state) ?? "listening"
        generation = try c.decodeIfPresent(UInt64.self, forKey: .generation) ?? 0
    }
}

public struct FabushiTalkStateResult: Decodable, Sendable, Equatable {
    public let sessionId: String
    public let state: String

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case state
    }
}

public struct FabushiTalkBargeInResult: Decodable, Sendable, Equatable {
    public let sessionId: String
    public let state: String
    public let flushedChunks: Int

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case state
        case flushedChunks = "flushed_chunks"
    }
}

public struct FabushiTalkEnqueueAudioResult: Decodable, Sendable, Equatable {
    public let bufferedChunks: Int
    public let state: String

    enum CodingKeys: String, CodingKey {
        case bufferedChunks = "buffered_chunks"
        case state
    }
}

public struct FabushiTalkDequeueAudioResult: Decodable, Sendable, Equatable {
    public let chunkB64: String?
    public let bufferedChunks: Int

    enum CodingKeys: String, CodingKey {
        case chunkB64 = "chunk_b64"
        case bufferedChunks = "buffered_chunks"
    }
}

public struct FabushiTalkVadResult: Decodable, Sendable, Equatable {
    public let bargeInTriggered: Bool
    public let aiMediaTapBargeIn: Bool
    public let silenceDetected: Bool
    public let consecutiveSpeechFrames: Int
    public let flushedChunks: Int

    enum CodingKeys: String, CodingKey {
        case bargeInTriggered = "barge_in_triggered"
        case aiMediaTapBargeIn = "ai_media_tap_barge_in"
        case silenceDetected = "silence_detected"
        case consecutiveSpeechFrames = "consecutive_speech_frames"
        case flushedChunks = "flushed_chunks"
    }
}

public struct FabushiTalkTurnRequest: Encodable, Sendable {
    public let sessionId: String?
    public let chatId: Int64?
    public let botId: UInt64?
    public let transcript: String
    public let voicePreset: String?
    public let bargeIn: Bool?

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case chatId = "chat_id"
        case botId = "bot_id"
        case transcript
        case voicePreset = "voice_preset"
        case bargeIn = "barge_in"
    }

    public init(
        sessionId: String? = nil,
        chatId: Int64? = nil,
        botId: UInt64? = nil,
        transcript: String,
        voicePreset: String? = nil,
        bargeIn: Bool? = nil
    ) {
        self.sessionId = sessionId
        self.chatId = chatId
        self.botId = botId
        self.transcript = transcript
        self.voicePreset = voicePreset
        self.bargeIn = bargeIn
    }
}

public struct FabushiTalkTurnResponse: Decodable, Sendable, Equatable {
    public let sessionId: String
    public let callId: String?
    public let state: String
    public let transcript: String
    public let replyText: String
    public let ttsSegments: [String]
    public let botId: UInt64?
    public let chatId: Int64?

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case callId = "call_id"
        case state
        case transcript
        case replyText = "reply_text"
        case ttsSegments = "tts_segments"
        case botId = "bot_id"
        case chatId = "chat_id"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try c.decodeIfPresent(String.self, forKey: .sessionId) ?? ""
        callId = try c.decodeIfPresent(String.self, forKey: .callId)
        state = try c.decodeIfPresent(String.self, forKey: .state) ?? "speaking"
        transcript = try c.decodeIfPresent(String.self, forKey: .transcript) ?? ""
        replyText = try c.decodeIfPresent(String.self, forKey: .replyText) ?? ""
        ttsSegments = try c.decodeIfPresent([String].self, forKey: .ttsSegments) ?? []
        botId = try c.decodeIfPresent(UInt64.self, forKey: .botId)
        chatId = try c.decodeIfPresent(Int64.self, forKey: .chatId)
    }
}

public struct FabushiTalkMessageEntry: Identifiable, Sendable, Equatable {
    public let id: UUID
    public let isUser: Bool
    public let text: String
    public let timestamp: Date

    public init(id: UUID = UUID(), isUser: Bool, text: String, timestamp: Date = Date()) {
        self.id = id
        self.isUser = isUser
        self.text = text
        self.timestamp = timestamp
    }
}
