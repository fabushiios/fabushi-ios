import AVFoundation
import Foundation
@preconcurrency import LiveKitWebRTC
import Observation
@preconcurrency import PushKit
import UIKit

/// Generation-fenced controller orchestrating 1:1 & Group Calls over existing native Stratum A
/// owners (`HumanCallSystemCoordinator`, `HumanCallMediaPort`, `HumanCallPeerConnection`,
/// `HumanCallBroadcastIPC`) and real Rust `calls.*` RPCs (`WebRtcCallEngine` + server signaling).
@MainActor
@Observable
public final class FabushiCallSessionController {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let launchConfiguration: FabushiLaunchConfiguration

    // Call State
    public private(set) var callState: FabushiCallSessionState = .idle
    public private(set) var activeCallId: String? = nil
    public private(set) var activePeerUserId: UInt64? = nil
    public private(set) var activePeerTitle: String = ""
    public private(set) var isVideoCall: Bool = false
    public private(set) var isMuted: Bool = false
    public private(set) var isCameraEnabled: Bool = false
    public private(set) var isScreenSharing: Bool = false
    public private(set) var emojiFingerprint: String? = nil
    public private(set) var audioLevel: Double = 0
    public private(set) var isSpeaking: Bool = false
    public private(set) var elapsedSeconds: Int = 0

    // Video Tracks (LiveKitWebRTC owned strictly on MainActor)
    internal private(set) var localVideoTrack: LKRTCVideoTrack? = nil
    internal private(set) var remoteVideoTrack: LKRTCVideoTrack? = nil

    // Permission, ReplayKit & Honest BLK-02 Disclosure Flags
    public private(set) var isPermissionDenied: Bool = false
    public private(set) var isScreenShareUnavailable: Bool = false
    /// Production server-side WebRTC NAT/SFU & dual-device media verification remain blocked under BLK-02.
    public let isBlk02SfuDisclosureVisible: Bool = true
    public private(set) var voipTokenRegistered: Bool = false

    // Group SFU Room & Call History
    public private(set) var activeGroupRoomChatId: Int64? = nil
    public private(set) var groupRoomSubscribers: [UInt64] = []
    public private(set) var callHistory: [FabushiCallRecordDto] = []
    public private(set) var pendingRatingCallId: String? = nil
    public var isRatingSheetPresented: Bool = false

    // Status
    public private(set) var isLoading: Bool = false
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var observedGeneration: UInt64

    @ObservationIgnored private let mediaPort: HumanCallMediaPort
    @ObservationIgnored private var peerConnection: HumanCallPeerConnection?
    @ObservationIgnored private var activeBroadcastSessionId: String?
    @ObservationIgnored private let purgerId = "m5.calls.\(UUID().uuidString)"
    @ObservationIgnored nonisolated(unsafe) private var eventTask: Task<Void, Never>?
    @ObservationIgnored nonisolated(unsafe) private var durationTimerTask: Task<Void, Never>?
    @ObservationIgnored nonisolated(unsafe) private var interruptionObserver: NSObjectProtocol?

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration = .current
    ) {
        self.coordinator = coordinator
        self.router = router
        self.launchConfiguration = launchConfiguration
        self.observedGeneration = coordinator.accountScopeGeneration
        self.mediaPort = HumanCallMediaPort(defaults: .standard)

        // Honor deterministic launch-configuration flags for permission & ReplayKit failure paths.
        if launchConfiguration.simulateCallPermissionDenied {
            self.isPermissionDenied = true
        }
        if launchConfiguration.simulateReplayKitDenied || !mediaPort.screenShareCapability().available {
            self.isScreenShareUnavailable = true
        }
        self.voipTokenRegistered = !(UserDefaults.standard.string(forKey: HumanCallSystemCoordinator.voIPTokenDefaultsKey)?.isEmpty ?? true)

        installLifecycleHooks()
        startEventSubscription()
        installAudioInterruptionObserver()
    }

    deinit {
        eventTask?.cancel()
        durationTimerTask?.cancel()
        if let observer = interruptionObserver {
            NotificationCenter.default.removeObserver(observer)
        }
    }

    // MARK: - Lifecycle, CallKit Binding & Account-Switch Teardown

    private func installLifecycleHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] _ in
            self?.teardownForAccountScopeChange()
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            self?.teardownForAccountScopeChange()
        }
    }

    public func bindSystemCallCoordinator() {
        let systemCoordinator = HumanCallSystemCoordinator.shared
        systemCoordinator.start()
        systemCoordinator.bind { [weak self] action in
            guard let self else { return }
            await self.handleSystemCallAction(action)
        }
        systemCoordinator.bindVoIPTokenChangeHandler { [weak self] in
            guard let self else { return }
            let token = UserDefaults.standard.string(forKey: HumanCallSystemCoordinator.voIPTokenDefaultsKey)
            self.voipTokenRegistered = !(token?.isEmpty ?? true)
        }
    }

    public func detachLifecycleHooks() {
        eventTask?.cancel()
        eventTask = nil
        durationTimerTask?.cancel()
        durationTimerTask = nil
        if let observer = interruptionObserver {
            NotificationCenter.default.removeObserver(observer)
            interruptionObserver = nil
        }
        HumanCallSystemCoordinator.shared.unbind()
        teardownPeerConnectionAndBroadcast()
        coordinator.unregisterAccountSecretsPurger(id: purgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: purgerId)
    }

    public func reconcileAccountScopeIfNeeded() {
        let current = coordinator.accountScopeGeneration
        guard current != observedGeneration else { return }
        teardownForAccountScopeChange()
    }

    private func teardownForAccountScopeChange() {
        observedGeneration = coordinator.accountScopeGeneration
        durationTimerTask?.cancel()
        durationTimerTask = nil
        HumanCallSystemCoordinator.shared.unbind()
        teardownPeerConnectionAndBroadcast()
        callState = .idle
        activeCallId = nil
        activePeerUserId = nil
        activePeerTitle = ""
        isVideoCall = false
        isMuted = false
        isCameraEnabled = false
        isScreenSharing = false
        emojiFingerprint = nil
        audioLevel = 0
        isSpeaking = false
        elapsedSeconds = 0
        activeGroupRoomChatId = nil
        groupRoomSubscribers = []
        callHistory = []
        pendingRatingCallId = nil
        isRatingSheetPresented = false
        lastErrorMessage = nil
        router.activeCallPip = nil
    }

    private func teardownPeerConnectionAndBroadcast() {
        if let sessionId = activeBroadcastSessionId {
            HumanCallBroadcastIPC.endSession(sessionId)
            activeBroadcastSessionId = nil
        }
        peerConnection?.close()
        peerConnection = nil
        localVideoTrack = nil
        remoteVideoTrack = nil
        isScreenSharing = false
    }

    private func installAudioInterruptionObserver() {
        interruptionObserver = NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            let shouldRecover = HumanCallMediaPort.shouldRecoverAfterAudioInterruption(
                userInfo: notification.userInfo
            )
            guard shouldRecover else { return }
            Task { @MainActor [weak self] in
                guard let self, self.callState.isActive else { return }
                _ = try? self.mediaPort.applyPreferredMicrophone()
            }
        }
    }

    private func startEventSubscription() {
        eventTask?.cancel()
        let stream = coordinator.host.makeEventStream()
        eventTask = Task { @MainActor [weak self] in
            for await event in stream {
                guard !Task.isCancelled, let self else { break }
                guard event.generation >= self.coordinator.accountScopeGeneration else { continue }
                if case .callUpdated(let callId, let state, _) = event.kind {
                    self.applyRemoteCallStateUpdate(callId: callId, rawState: state)
                }
            }
        }
    }

    private func handleSystemCallAction(_ action: HumanCallSystemAction) async {
        switch action {
        case .incoming(let descriptor):
            await receiveIncomingCall(
                callId: descriptor.callId,
                callerUserId: 0,
                displayName: descriptor.displayName,
                isVideo: descriptor.hasVideo
            )
        case .answer(let descriptor):
            if activeCallId == nil {
                activeCallId = descriptor.callId
                activePeerTitle = descriptor.displayName
                isVideoCall = descriptor.hasVideo
            }
            await acceptIncomingCall(locale: .en)
        case .end:
            await hangup(reason: "callkit_ended", locale: .en)
        }
    }

    private func applyRemoteCallStateUpdate(callId: String, rawState: String) {
        guard activeCallId == nil || activeCallId == callId else { return }
        switch rawState.lowercased() {
        case "ringing":
            callState = .ringingIncoming
        case "connecting":
            callState = .connecting
        case "active", "connected":
            callState = .connected
            startDurationTimerIfNeeded()
        case "ended", "declined", "hung_up":
            let finishedId = activeCallId ?? callId
            teardownPeerConnectionAndBroadcast()
            callState = .ended
            durationTimerTask?.cancel()
            router.activeCallPip = nil
            pendingRatingCallId = finishedId
            isRatingSheetPresented = true
        default:
            break
        }
        syncCallStoreAndPip(locale: .en)
    }

    // MARK: - Permissions & Capabilities

    public func refreshCapabilities() {
        if launchConfiguration.simulateCallPermissionDenied {
            isPermissionDenied = true
        }
        if launchConfiguration.simulateReplayKitDenied {
            isScreenShareUnavailable = true
        } else {
            isScreenShareUnavailable = !mediaPort.screenShareCapability().available
        }
    }

    public func openSystemSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }

    // MARK: - Call History (`calls.list`)

    public func loadCallHistory() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isLoading = true
        defer { isLoading = false }

        do {
            let records: [FabushiCallRecordDto] = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("list"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            callHistory = records
            lastErrorMessage = nil
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }
    }

    // MARK: - Outgoing & Incoming Call Flows (`calls.invite`, `calls.accept`, `calls.decline`, `calls.hangup`, `calls.engine*`)

    @discardableResult
    public func startOutgoingCall(
        calleeUserId: UInt64,
        peerTitle: String,
        isVideo: Bool,
        chatId: Int64? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        lastErrorMessage = nil

        if launchConfiguration.simulateCallPermissionDenied {
            isPermissionDenied = true
            callState = .failed
            lastErrorMessage = FabushiPlatformStrings.text(.callsPermissionDeniedBody, locale: locale)
            return false
        }

        let permissions = await mediaPort.requestPermissions(audio: true, video: isVideo)
        guard coordinator.accountScopeGeneration == capturedGen else { return false }
        if permissions.microphone == .denied || (isVideo && permissions.camera == .denied) {
            isPermissionDenied = true
            callState = .failed
            lastErrorMessage = FabushiPlatformStrings.text(.callsPermissionDeniedBody, locale: locale)
            return false
        }
        isPermissionDenied = false

        isLoading = true
        defer { isLoading = false }

        activePeerUserId = calleeUserId
        activePeerTitle = peerTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? "User #\(calleeUserId)"
            : peerTitle
        isVideoCall = isVideo
        isCameraEnabled = isVideo
        isMuted = false
        elapsedSeconds = 0
        callState = .ringingOutgoing

        let localUserId = currentUserId()
        var resolvedCallId = "call_\(Int64(Date().timeIntervalSince1970 * 1000))"

        // 1. Dispatch server invite when authenticated/reachable; fall back to local Rust engine cleanly.
        do {
            let inviteRecord: FabushiCallRecordDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("invite"),
                params: FabushiCallInviteRequest(
                    calleeId: calleeUserId,
                    isVideo: isVideo,
                    chatId: chatId
                )
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            resolvedCallId = inviteRecord.callId
            if !callHistory.contains(where: { $0.callId == inviteRecord.callId }) {
                callHistory.insert(inviteRecord, at: 0)
            }
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            // Keep local WebRtcCallEngine session alive for offline/sandbox signaling inspection.
            lastErrorMessage = error.localizedDescription
        }

        activeCallId = resolvedCallId

        // 2. Start pure-Rust WebRtcCallEngine outgoing state machine & compute 4-emoji fingerprint.
        let numericCallId = Int64(resolvedCallId.filter(\.isNumber)) ?? Int64(abs(resolvedCallId.hashValue))
        do {
            struct EngineStartParams: Encodable, Sendable {
                let call_id: Int64
                let local_user_id: UInt64
                let remote_user_id: UInt64
                let is_video: Bool
            }
            let engineResult: FabushiCallEngineStartResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("engineStartOutgoing"),
                params: EngineStartParams(
                    call_id: numericCallId,
                    local_user_id: localUserId,
                    remote_user_id: calleeUserId,
                    is_video: isVideo
                )
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            if let sdp = engineResult.localSdp, !sdp.isEmpty {
                _ = try? await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.calls.method("signal"),
                    params: FabushiCallSignalRequest(
                        callId: resolvedCallId,
                        signalType: "offer",
                        sdp: sdp,
                        targetUserId: calleeUserId
                    )
                ) as FabushiJSONValue
            }
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
        }

        // 3. Configure native LiveKitWebRTC peer connection using TURN credentials if available.
        await configureLiveKitPeerConnectionIfPossible(enableVideo: isVideo, expectedGeneration: capturedGen)

        // 4. Refresh E2EE 4-emoji fingerprint from Rust engine.
        await refreshEmojiFingerprint()

        callState = .connecting
        syncCallStoreAndPip(locale: locale)
        return true
    }

    public func receiveIncomingCall(
        callId: String,
        callerUserId: UInt64,
        displayName: String,
        isVideo: Bool,
        locale: FabushiLocale = .en
    ) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        activeCallId = callId
        activePeerUserId = callerUserId
        activePeerTitle = displayName
        isVideoCall = isVideo
        isCameraEnabled = isVideo
        callState = .ringingIncoming

        struct IncomingParams: Encodable, Sendable {
            let call_id: Int64
            let caller_user_id: UInt64
            let callee_user_id: UInt64
            let is_video: Bool
        }
        let numericCallId = Int64(callId.filter(\.isNumber)) ?? Int64(abs(callId.hashValue))
        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.calls.method("engineReceiveIncoming"),
            params: IncomingParams(
                call_id: numericCallId,
                caller_user_id: callerUserId,
                callee_user_id: currentUserId(),
                is_video: isVideo
            )
        ) as FabushiJSONValue

        guard coordinator.accountScopeGeneration == capturedGen else { return }
        syncCallStoreAndPip(locale: locale)
    }

    public func acceptIncomingCall(locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        guard let callId = activeCallId else { return }
        let capturedGen = coordinator.accountScopeGeneration

        if launchConfiguration.simulateCallPermissionDenied {
            isPermissionDenied = true
            callState = .failed
            return
        }

        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.calls.method("accept"),
            params: FabushiCallIdParams(callId: callId)
        ) as FabushiCallRecordDto

        if let accepted: FabushiCallEngineAcceptResult = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.calls.method("engineAcceptIncoming"),
            params: FabushiEmptyParams()
        ) {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            if let fp = accepted.emojiFingerprint, !fp.isEmpty {
                emojiFingerprint = fp
            }
        }

        await configureLiveKitPeerConnectionIfPossible(enableVideo: isVideoCall, expectedGeneration: capturedGen)
        await refreshEmojiFingerprint()

        callState = .connected
        startDurationTimerIfNeeded()
        syncCallStoreAndPip(locale: locale)
    }

    public func declineIncomingCall(locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        guard let callId = activeCallId else { return }
        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.calls.method("decline"),
            params: FabushiCallIdParams(callId: callId)
        ) as FabushiJSONValue
        await hangup(reason: "declined", showRating: false, locale: locale)
    }

    public func hangup(
        reason: String = "local_hangup",
        showRating: Bool = true,
        locale: FabushiLocale = .en
    ) async {
        reconcileAccountScopeIfNeeded()
        let finishedCallId = activeCallId
        durationTimerTask?.cancel()
        durationTimerTask = nil
        teardownPeerConnectionAndBroadcast()

        if let callId = finishedCallId {
            _ = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("hangup"),
                params: FabushiCallIdParams(callId: callId)
            ) as FabushiJSONValue
        }

        struct EngineHangupParams: Encodable, Sendable {
            let reason: String
        }
        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.calls.method("engineHangup"),
            params: EngineHangupParams(reason: reason)
        ) as FabushiJSONValue

        callState = .ended
        router.activeCallPip = nil

        if showRating, let finishedCallId {
            pendingRatingCallId = finishedCallId
            isRatingSheetPresented = true
        }
        activeCallId = nil
        syncCallStoreAndPip(locale: locale)
    }

    public func presentRatingSheet(for callId: String) {
        pendingRatingCallId = callId
        isRatingSheetPresented = true
    }

    // MARK: - Signaling Frame, Fingerprint, Audio Level & Post-Call Rating

    @discardableResult
    public func applyEngineSignalFrame(_ frame: FabushiJSONValue) async -> FabushiCallEngineApplySignalResult? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let result: FabushiCallEngineApplySignalResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("engineApplySignal"),
                params: frame
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if let fp = result.emojiFingerprint, !fp.isEmpty {
                emojiFingerprint = fp
            }
            return result
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    @discardableResult
    public func refreshEmojiFingerprint(sha256Hex: String? = nil) async -> String? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct FingerprintParams: Encodable, Sendable {
            let sha256_hex: String?
        }

        do {
            let result: FabushiCallEmojiFingerprintResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("engineEmojiFingerprint"),
                params: FingerprintParams(sha256_hex: sha256Hex)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            if let emojis = result.emojis, !emojis.isEmpty {
                emojiFingerprint = emojis
            }
            return result.emojis
        } catch {
            return nil
        }
    }

    @discardableResult
    public func pushAudioLevel(pcm: [Int16]) async -> FabushiCallAudioLevelResult? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct AudioLevelParams: Encodable, Sendable {
            let pcm_i16: [Int16]
        }

        do {
            let res: FabushiCallAudioLevelResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("enginePushAudioLevel"),
                params: AudioLevelParams(pcm_i16: pcm)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            audioLevel = res.level
            isSpeaking = res.isSpeaking
            return res
        } catch {
            return nil
        }
    }

    @discardableResult
    public func submitCallRating(
        callId: String,
        rating: UInt8,
        feedback: String?,
        locale: FabushiLocale = .en
    ) async -> Bool {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        do {
            let updated: FabushiCallRecordDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("rate"),
                params: FabushiCallRateRequest(callId: callId, rating: rating, feedback: feedback)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            if let idx = callHistory.firstIndex(where: { $0.callId == updated.callId }) {
                callHistory[idx] = updated
            } else {
                callHistory.insert(updated, at: 0)
            }
            isRatingSheetPresented = false
            pendingRatingCallId = nil
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.callsRateSubmittedToast, locale: locale),
                isError: false
            )
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    // MARK: - Media Controls (Mute, Camera, ReplayKit Screen Share, PiP)

    public func toggleMute(locale: FabushiLocale = .en) {
        isMuted.toggle()
        peerConnection?.setMuted(isMuted)
        syncCallStoreAndPip(locale: locale)
    }

    public func toggleCamera(locale: FabushiLocale = .en) async {
        if launchConfiguration.simulateCallPermissionDenied {
            isPermissionDenied = true
            return
        }
        let target = !isCameraEnabled
        if target {
            let perms = await mediaPort.requestPermissions(audio: false, video: true)
            if perms.camera == .denied {
                isPermissionDenied = true
                return
            }
        }
        do {
            try await peerConnection?.setCameraEnabled(
                target,
                preferredCameraId: mediaPort.resolvedPreferences().cameraId
            )
            isCameraEnabled = target
            isVideoCall = isVideoCall || target
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func toggleScreenShare(locale: FabushiLocale = .en) async {
        if launchConfiguration.simulateReplayKitDenied || !mediaPort.screenShareCapability().available {
            isScreenShareUnavailable = true
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.callsScreenShareUnavailableBody, locale: locale),
                isError: true
            )
            return
        }

        let target = !isScreenSharing
        if target {
            let sessionId = "fabushi-broadcast-\(UUID().uuidString.lowercased())"
            activeBroadcastSessionId = sessionId
            HumanCallBroadcastIPC.beginSession(sessionId)
            do {
                try await peerConnection?.setExternalScreenShareEnabled(true, sessionID: sessionId)
                isScreenSharing = true
                isScreenShareUnavailable = false
            } catch {
                HumanCallBroadcastIPC.endSession(sessionId)
                activeBroadcastSessionId = nil
                isScreenSharing = false
                isScreenShareUnavailable = true
                lastErrorMessage = error.localizedDescription
            }
        } else {
            if let sessionId = activeBroadcastSessionId {
                HumanCallBroadcastIPC.endSession(sessionId)
                activeBroadcastSessionId = nil
            }
            try? await peerConnection?.setScreenShareEnabled(false)
            isScreenSharing = false
        }
    }

    public func minimizeToFloatingPip(locale: FabushiLocale = .en) {
        guard let callId = activeCallId, callState.isActive else { return }
        router.activeCallPip = CallPipOverlayState(
            id: callId,
            peerTitle: activePeerTitle.isEmpty ? "Fabushi Call" : activePeerTitle,
            stateText: callState.localizedTitle(locale: locale),
            isMuted: isMuted
        )
        router.dismissActiveModalSheet(coordinator: coordinator)
    }

    // MARK: - Group SFU Room (`calls.sfuJoinRoom` / `calls.sfuLeaveRoom`)

    @discardableResult
    public func joinGroupRoom(chatId: Int64) async -> [UInt64] {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct SfuParams: Encodable, Sendable {
            let chat_id: Int64
            let user_id: UInt64
        }

        do {
            let res: FabushiCallSfuRoomResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("sfuJoinRoom"),
                params: SfuParams(chat_id: chatId, user_id: max(1, currentUserId()))
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return [] }
            activeGroupRoomChatId = res.chatId
            groupRoomSubscribers = res.subscribers
            return res.subscribers
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return [] }
            lastErrorMessage = error.localizedDescription
            return []
        }
    }

    @discardableResult
    public func leaveGroupRoom() async -> [UInt64] {
        reconcileAccountScopeIfNeeded()
        guard let chatId = activeGroupRoomChatId else { return [] }
        let capturedGen = coordinator.accountScopeGeneration

        struct SfuParams: Encodable, Sendable {
            let chat_id: Int64
            let user_id: UInt64
        }

        do {
            let res: FabushiCallSfuRoomResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("sfuLeaveRoom"),
                params: SfuParams(chat_id: chatId, user_id: max(1, currentUserId()))
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return [] }
            activeGroupRoomChatId = nil
            groupRoomSubscribers = res.subscribers
            return res.subscribers
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return [] }
            lastErrorMessage = error.localizedDescription
            return []
        }
    }

    // MARK: - Private LiveKitWebRTC Configuration Helpers

    private func configureLiveKitPeerConnectionIfPossible(
        enableVideo: Bool,
        expectedGeneration: UInt64
    ) async {
        var iceServers: [HumanCallPeerConnection.IceServer] = []
        if let rawTurn: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.calls.method("turnCredentials"),
            params: FabushiEmptyParams()
        ) {
            iceServers = Self.parseIceServers(from: rawTurn)
        }
        guard coordinator.accountScopeGeneration == expectedGeneration else { return }

        let pc = HumanCallPeerConnection()
        pc.onLocalVideoTrack = { [weak self] track in
            self?.localVideoTrack = track
        }
        pc.onRemoteVideoTrack = { [weak self] track in
            self?.remoteVideoTrack = track
        }
        pc.onScreenShareChange = { [weak self] sharing in
            self?.isScreenSharing = sharing
        }
        pc.onScreenShareFailure = { [weak self] message in
            self?.isScreenShareUnavailable = true
            self?.lastErrorMessage = message
        }
        pc.onStateChange = { [weak self] state in
            guard let self else { return }
            switch state {
            case .connected:
                self.callState = .connected
                self.startDurationTimerIfNeeded()
            case .disconnected:
                self.callState = .reconnecting
            case .failed:
                self.callState = .failed
            case .closed:
                if self.callState.isActive {
                    self.callState = .ended
                }
            default:
                break
            }
        }
        pc.onLocalCandidate = { [weak self] candidatePayload in
            guard let self, let callId = self.activeCallId else { return }
            let candidate = candidatePayload["candidate"] as? String
            let sdpMid = candidatePayload["sdpMid"] as? String
            let mlineIndex = (candidatePayload["sdpMLineIndex"] as? Int).map(UInt32.init)
            let targetUserId = self.activePeerUserId
            Task { @MainActor [weak self] in
                guard let self else { return }
                _ = try? await self.coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.calls.method("signal"),
                    params: FabushiCallSignalRequest(
                        callId: callId,
                        signalType: "ice_candidate",
                        candidate: candidate,
                        sdpMid: sdpMid,
                        sdpMlineIndex: mlineIndex,
                        targetUserId: targetUserId
                    )
                ) as FabushiJSONValue
            }
        }

        do {
            try await pc.configure(
                iceServers: iceServers,
                enableVideo: enableVideo,
                preferredCameraId: mediaPort.resolvedPreferences().cameraId
            )
            guard coordinator.accountScopeGeneration == expectedGeneration else {
                pc.close()
                return
            }
            peerConnection = pc
            pc.setMuted(isMuted)
        } catch {
            pc.close()
            // Simulator or sandboxed test hosts may lack physical audio/video hardware; keep Rust signaling live.
            lastErrorMessage = error.localizedDescription
        }
    }

    private static func parseIceServers(from json: FabushiJSONValue) -> [HumanCallPeerConnection.IceServer] {
        let candidates = json["ice_servers"]?.arrayValue
            ?? json["iceServers"]?.arrayValue
            ?? json.arrayValue
            ?? []

        var parsed: [HumanCallPeerConnection.IceServer] = []
        for item in candidates {
            guard let obj = item.objectValue else { continue }
            var raw: [String: Any] = [:]
            if let urlStr = obj["urls"]?.stringValue ?? obj["url"]?.stringValue {
                raw["urls"] = urlStr
            } else if let urlArr = obj["urls"]?.arrayValue {
                raw["urls"] = urlArr.compactMap(\.stringValue)
            }
            if let user = obj["username"]?.stringValue {
                raw["username"] = user
            }
            if let cred = obj["credential"]?.stringValue {
                raw["credential"] = cred
            }
            if let server = HumanCallPeerConnection.IceServer(raw: raw) {
                parsed.append(server)
            }
        }
        return parsed
    }

    private func startDurationTimerIfNeeded() {
        guard durationTimerTask == nil else { return }
        durationTimerTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard !Task.isCancelled, let self, self.callState == .connected else { break }
                self.elapsedSeconds += 1
            }
        }
    }

    private func syncCallStoreAndPip(locale: FabushiLocale) {
        if let callId = activeCallId, callState.isActive {
            let syntheticEvent = FabushiDomainEvent(
                seq: 0,
                generation: coordinator.accountScopeGeneration,
                timestampMs: Int64(Date().timeIntervalSince1970 * 1000),
                type: "call_updated",
                rawPayloadJSON: "{}",
                kind: .callUpdated(callId: callId, state: callState.rawValue, payload: .null)
            )
            coordinator.callSessionStore.handleEvent(syntheticEvent)
            if router.activeCallPip != nil {
                router.activeCallPip = CallPipOverlayState(
                    id: callId,
                    peerTitle: activePeerTitle.isEmpty ? "Fabushi Call" : activePeerTitle,
                    stateText: callState.localizedTitle(locale: locale),
                    isMuted: isMuted
                )
            }
        } else {
            coordinator.callSessionStore.resetForAccountScopeChange(
                generation: coordinator.accountScopeGeneration
            )
        }
    }

    private func currentUserId() -> UInt64 {
        if case .authenticated(let userId, _, _) = coordinator.authState, userId > 0 {
            return userId
        }
        return 1
    }
}
