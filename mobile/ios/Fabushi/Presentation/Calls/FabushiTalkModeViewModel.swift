import AVFoundation
import Foundation
import Observation

/// Generation-fenced ViewModel driving Full-Duplex AI Talk Mode (`Feature #29`):
/// - `AVAudioEngine` hardware AEC (`setVoiceProcessingEnabled(true)`, gracefully guarded on simulator)
/// - Existing standalone `OfflineSpeechTranscriber` (on-device Apple Speech ASR) + `VoiceRecorder`
/// - Real Rust `talk.*` RPCs (`createSession`, `transition`, `bargeIn`, `resumeListening`,
///   `enqueueAudio`, `dequeueAudio`, `processVadPcm`, `turn`, `callInvite`, `close`).
@MainActor
@Observable
public final class FabushiTalkModeViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    public private(set) var sessionId: String? = nil
    public private(set) var state: FabushiTalkSessionState = .idle
    public private(set) var audioLevel: Double = 0
    public private(set) var bufferedChunksCount: Int = 0
    public private(set) var lastFlushedChunks: Int = 0
    public private(set) var lastBargeInElapsedMs: Double? = nil
    public private(set) var isHardwareAecActive: Bool = false
    public private(set) var isVoiceRecording: Bool = false
    public private(set) var isBusy: Bool = false
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var transcriptHistory: [FabushiTalkMessageEntry] = []
    public var draftTranscript: String = ""
    public private(set) var observedGeneration: UInt64

    @ObservationIgnored private let speechTranscriber = OfflineSpeechTranscriber()
    @ObservationIgnored private let voiceRecorder = VoiceRecorder()
    @ObservationIgnored private var audioEngine: AVAudioEngine?
    @ObservationIgnored private let purgerId = "m5.talk.\(UUID().uuidString)"

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
        self.observedGeneration = coordinator.accountScopeGeneration
        installAccountResetHooks()
    }

    // MARK: - Lifecycle & Generation Fencing

    private func installAccountResetHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] _ in
            self?.resetForAccountScopeChange()
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            self?.resetForAccountScopeChange()
        }
    }

    public func detachLifecycleHooks() {
        stopAudioEngine()
        speechTranscriber.cancel()
        voiceRecorder.cancel()
        coordinator.unregisterAccountSecretsPurger(id: purgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: purgerId)
    }

    public func reconcileAccountScopeIfNeeded() {
        let current = coordinator.accountScopeGeneration
        guard current != observedGeneration else { return }
        resetForAccountScopeChange()
    }

    private func resetForAccountScopeChange() {
        observedGeneration = coordinator.accountScopeGeneration
        stopAudioEngine()
        speechTranscriber.cancel()
        voiceRecorder.cancel()
        sessionId = nil
        state = .idle
        audioLevel = 0
        bufferedChunksCount = 0
        lastFlushedChunks = 0
        lastBargeInElapsedMs = nil
        isVoiceRecording = false
        isBusy = false
        lastErrorMessage = nil
        transcriptHistory = []
        draftTranscript = ""
    }

    // MARK: - Session Lifecycle (`talk.createSession`, `talk.transition`, `talk.close`)

    @discardableResult
    public func startSession(remote: Bool = false, enableHardwareAec: Bool = true) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isBusy = true
        lastErrorMessage = nil
        defer { isBusy = false }

        if enableHardwareAec {
            configureHardwareAecSafely()
        }

        struct CreateParams: Encodable, Sendable {
            let remote: Bool
        }

        do {
            let res: FabushiTalkSessionCreateResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.talk.method("createSession"),
                params: CreateParams(remote: remote)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            sessionId = res.sessionId
            state = FabushiTalkSessionState(rawValue: res.state) ?? .listening
            bufferedChunksCount = 0
            lastFlushedChunks = 0
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            stopAudioEngine()
            return false
        }
    }

    @discardableResult
    public func transition(to nextState: FabushiTalkSessionState) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct TransitionParams: Encodable, Sendable {
            let state: String
        }

        do {
            let res: FabushiTalkStateResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.talk.method("transition"),
                params: TransitionParams(state: nextState.rawValue)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            sessionId = res.sessionId
            state = FabushiTalkSessionState(rawValue: res.state) ?? nextState
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    @discardableResult
    public func triggerBargeIn() async -> FabushiTalkBargeInResult? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        let clock = ContinuousClock()
        let start = clock.now

        do {
            let res: FabushiTalkBargeInResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.talk.method("bargeIn"),
                params: FabushiEmptyParams()
            )
            let elapsed = start.duration(to: clock.now)
            let elapsedMs = Double(elapsed.components.seconds) * 1_000.0
                + Double(elapsed.components.attoseconds) / 1_000_000_000_000_000.0
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastBargeInElapsedMs = elapsedMs
            lastFlushedChunks = res.flushedChunks
            bufferedChunksCount = 0
            sessionId = res.sessionId
            state = FabushiTalkSessionState(rawValue: res.state) ?? .interrupted
            return res
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    @discardableResult
    public func resumeListening() async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let res: FabushiTalkStateResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.talk.method("resumeListening"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            sessionId = res.sessionId
            state = FabushiTalkSessionState(rawValue: res.state) ?? .listening
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            return false
        }
    }

    public func closeSession() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        stopAudioEngine()
        speechTranscriber.cancel()
        voiceRecorder.cancel()
        isVoiceRecording = false

        if let res: FabushiTalkStateResult = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.talk.method("close"),
            params: FabushiEmptyParams()
        ) {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            sessionId = res.sessionId
            state = FabushiTalkSessionState(rawValue: res.state) ?? .closed
        } else {
            state = .closed
        }
        bufferedChunksCount = 0
        audioLevel = 0
    }

    // MARK: - Audio Buffer & VAD PCM Barge-In (`talk.enqueueAudio`, `talk.dequeueAudio`, `talk.processVadPcm`)

    @discardableResult
    public func enqueueAudioChunk(_ data: Data) async throws -> FabushiTalkEnqueueAudioResult {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct EnqueueParams: Encodable, Sendable {
            let chunk_b64: String
        }

        let res: FabushiTalkEnqueueAudioResult = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.talk.method("enqueueAudio"),
            params: EnqueueParams(chunk_b64: data.base64EncodedString())
        )
        guard coordinator.accountScopeGeneration == capturedGen else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: capturedGen,
                current: coordinator.accountScopeGeneration
            )
        }
        bufferedChunksCount = res.bufferedChunks
        if let parsed = FabushiTalkSessionState(rawValue: res.state) {
            state = parsed
        }
        return res
    }

    @discardableResult
    public func dequeueAudioChunk() async throws -> FabushiTalkDequeueAudioResult {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        let res: FabushiTalkDequeueAudioResult = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.talk.method("dequeueAudio"),
            params: FabushiEmptyParams()
        )
        guard coordinator.accountScopeGeneration == capturedGen else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: capturedGen,
                current: coordinator.accountScopeGeneration
            )
        }
        bufferedChunksCount = res.bufferedChunks
        return res
    }

    @discardableResult
    public func processVadPcmFrame(_ pcm: [Int16]) async throws -> FabushiTalkVadResult {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        let clock = ContinuousClock()
        let start = clock.now

        struct VadParams: Encodable, Sendable {
            let pcm_i16: [Int16]
        }

        let res: FabushiTalkVadResult = try await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.talk.method("processVadPcm"),
            params: VadParams(pcm_i16: pcm)
        )
        let elapsed = start.duration(to: clock.now)
        let elapsedMs = Double(elapsed.components.seconds) * 1_000.0
            + Double(elapsed.components.attoseconds) / 1_000_000_000_000_000.0

        guard coordinator.accountScopeGeneration == capturedGen else {
            throw FabushiCoordinatorError.staleGeneration(
                captured: capturedGen,
                current: coordinator.accountScopeGeneration
            )
        }

        audioLevel = Self.normalizedRmsLevel(pcm: pcm)
        if res.bargeInTriggered {
            lastBargeInElapsedMs = elapsedMs
            lastFlushedChunks = res.flushedChunks
            bufferedChunksCount = 0
            state = .interrupted
        }
        return res
    }

    // MARK: - Voice Capture -> Offline Apple Speech ASR -> `talk.turn`

    public func toggleVoiceCaptureAndTranscribe(locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        if !isVoiceRecording {
            await voiceRecorder.start()
            if let err = voiceRecorder.errorMessage {
                lastErrorMessage = err
                isVoiceRecording = false
                router.toastController.showNonChatToast(err, isError: true)
            } else {
                isVoiceRecording = true
                if !state.isRunning {
                    _ = await startSession(remote: false)
                }
            }
            return
        }

        isVoiceRecording = false
        guard let recording = voiceRecorder.stop() else { return }
        defer { try? FileManager.default.removeItem(at: recording.url) }

        do {
            let speechLocale = locale == .zhHans ? Locale(identifier: "zh-CN") : Locale(identifier: "en-US")
            let transcript = try await speechTranscriber.transcribe(
                fileURL: recording.url,
                locale: speechLocale
            )
            await sendTranscriptTurn(transcript)
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    @discardableResult
    public func sendTranscriptTurn(
        _ rawTranscript: String? = nil,
        chatId: Int64? = nil,
        botId: UInt64? = nil
    ) async -> FabushiTalkTurnResponse? {
        reconcileAccountScopeIfNeeded()
        let text = (rawTranscript ?? draftTranscript).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        let capturedGen = coordinator.accountScopeGeneration

        if !state.isRunning {
            _ = await startSession(remote: false)
        }
        draftTranscript = ""
        transcriptHistory.append(FabushiTalkMessageEntry(isUser: true, text: text))
        _ = await transition(to: .thinking)

        isBusy = true
        defer { isBusy = false }

        do {
            let res: FabushiTalkTurnResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.talk.method("turn"),
                params: FabushiTalkTurnRequest(
                    sessionId: sessionId,
                    chatId: chatId,
                    botId: botId,
                    transcript: text,
                    bargeIn: state == .interrupted ? true : nil
                )
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if !res.replyText.isEmpty {
                transcriptHistory.append(FabushiTalkMessageEntry(isUser: false, text: res.replyText))
            }
            state = FabushiTalkSessionState(rawValue: res.state) ?? .speaking
            lastErrorMessage = nil
            return res
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            _ = await resumeListening()
            return nil
        }
    }

    @discardableResult
    public func sendTalkCallInvite(calleeId: UInt64, isVideo: Bool = false, chatId: Int64? = nil) async -> FabushiJSONValue? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct TalkCallInviteParams: Encodable, Sendable {
            let callee_id: UInt64
            let is_video: Bool
            let chat_id: Int64?
        }

        do {
            let res: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.talk.method("callInvite"),
                params: TalkCallInviteParams(callee_id: calleeId, is_video: isVideo, chat_id: chatId)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            return res
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    // MARK: - Hardware AEC Guarded Setup

    private func configureHardwareAecSafely() {
        stopAudioEngine()
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(
                .playAndRecord,
                mode: .voiceChat,
                options: [.defaultToSpeaker, .allowBluetooth]
            )
            try session.setActive(true)
            let engine = AVAudioEngine()
            try engine.inputNode.setVoiceProcessingEnabled(true)
            audioEngine = engine
            isHardwareAecActive = engine.inputNode.isVoiceProcessingEnabled
        } catch {
            // Simulator or restricted environments safely degrade to software fallback without crashing.
            audioEngine = nil
            isHardwareAecActive = false
        }
    }

    private func stopAudioEngine() {
        audioEngine?.stop()
        audioEngine = nil
        isHardwareAecActive = false
    }

    public static func normalizedRmsLevel(pcm: [Int16]) -> Double {
        guard !pcm.isEmpty else { return 0 }
        var sumSquares: Double = 0
        for sample in pcm {
            let normalized = Double(sample) / Double(Int16.max)
            sumSquares += normalized * normalized
        }
        return min(1.0, sqrt(sumSquares / Double(pcm.count)))
    }
}
