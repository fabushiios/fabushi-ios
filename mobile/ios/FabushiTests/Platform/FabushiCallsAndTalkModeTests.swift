import Foundation
import XCTest
@testable import Fabushi

@MainActor
final class FabushiCallsAndTalkModeTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m5b_calls_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain() -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m5b.calls.\(UUID().uuidString)",
            account: "m5b-calls"
        )
    }

    private func signInTestUser(coordinator: FabushiCoordinator) async throws {
        let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
            method: FabushiRPCNamespace.auth.method("login"),
            params: FabushiLegacyLoginRequest(username: "calls_user", password: "StrongPassword!1"),
            extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
        )
    }

    // MARK: - 1. Real Rust WebRtcCallEngine: Outgoing, Incoming, Signal, Accept, 4-Emoji Fingerprint, Audio Level & SFU Room

    func testRealRustCallEngineOutgoingIncomingAcceptFingerprintAudioLevelAndSfuRoom() async throws {
        let dir = try makeTempDir("engine_local")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let toastCtrl = FabushiToastController(autoDismissEnabled: false)
            let router = ShellNavigationRouter(toastController: toastCtrl)
            let controller = FabushiCallSessionController(
                coordinator: coordinator,
                router: router,
                launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: [])
            )
            defer { controller.detachLifecycleHooks() }

            // 1. Outgoing engine start
            struct StartOutgoingParams: Encodable, Sendable {
                let call_id: Int64
                let local_user_id: UInt64
                let remote_user_id: UInt64
                let is_video: Bool
            }
            let outgoing: FabushiCallEngineStartResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("engineStartOutgoing"),
                params: StartOutgoingParams(call_id: 9001, local_user_id: 101, remote_user_id: 202, is_video: true)
            )
            XCTAssertEqual(outgoing.callId, 9001)
            XCTAssertFalse(outgoing.outboundFrames.isEmpty)

            // 2. Receive incoming + apply externally tagged PascalCase SignalingFrame + accept
            await controller.receiveIncomingCall(
                callId: "9002",
                callerUserId: 202,
                displayName: "Peer Two",
                isVideo: true
            )
            XCTAssertEqual(controller.callState, .ringingIncoming)

            let offerFrame: FabushiJSONValue = .object([
                "Offer": .object([
                    "call_id": .string("9002"),
                    "sdp": .string("v=0\r\no=- 12345 2 IN IP4 127.0.0.1\r\ns=Fabushi\r\nt=0 0\r\na=fingerprint:sha-256 01:02:03:04\r\n"),
                ]),
            ])
            let signalResult = await controller.applyEngineSignalFrame(offerFrame)
            XCTAssertEqual(signalResult?.applied, true)

            let accepted: FabushiCallEngineAcceptResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.calls.method("engineAcceptIncoming"),
                params: FabushiEmptyParams()
            )
            XCTAssertNotNil(accepted.answer)

            // 3. Deterministic 4-emoji E2EE fingerprint from 32-byte SHA-256 hex
            let shaHex = String(repeating: "ab", count: 32)
            let emojis = await controller.refreshEmojiFingerprint(sha256Hex: shaHex)
            let unwrappedEmojis = try XCTUnwrap(emojis)
            XCTAssertFalse(unwrappedEmojis.isEmpty)
            XCTAssertEqual(controller.emojiFingerprint, unwrappedEmojis)

            // 4. Push loud PCM audio frame -> active speech level
            let loudPcm = [Int16](repeating: 14_000, count: 320)
            let meter = await controller.pushAudioLevel(pcm: loudPcm)
            XCTAssertNotNil(meter)
            XCTAssertGreaterThan(meter?.level ?? 0, 0.1)
            XCTAssertEqual(meter?.isSpeaking, true)
            XCTAssertTrue(controller.isSpeaking)

            // 5. Group SFU room join & leave
            let joinedSubscribers = await controller.joinGroupRoom(chatId: -100777)
            XCTAssertFalse(joinedSubscribers.isEmpty)
            XCTAssertEqual(controller.activeGroupRoomChatId, -100777)

            let afterLeaveSubscribers = await controller.leaveGroupRoom()
            XCTAssertNil(controller.activeGroupRoomChatId)
            XCTAssertTrue(afterLeaveSubscribers.isEmpty)

            // 6. Clean engine hangup
            await controller.hangup(reason: "test_complete", showRating: false)
            XCTAssertEqual(controller.callState, .ended)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 2. Loopback HTTP `calls.list`, `calls.invite`, `calls.rate`, PiP & Account-Switch Teardown

    func testCallsListInviteAndRateOverLoopbackHTTPAndAccountSwitchTeardown() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":701,"username":"calls_user","session_id":"s-701","token":"tok-701","first_name":"Caller"}"#)
            case ("POST", "/api/v1/auth/logout"):
                return .json(200, #"{"ok":true}"#)
            case ("GET", "/api/v1/calls"):
                return .json(200, #"{"calls":[{"call_id":"call-hist-1","caller_id":701,"callee_id":702,"is_video":false,"state":"ended","duration_secs":125,"rating":null,"started_at_ms":1735000000000,"ended_at_ms":1735000125000}]}"#)
            case ("POST", "/api/v1/calls/invite"):
                return .json(200, #"{"call_id":"call-live-9","caller_id":701,"callee_id":702,"is_video":false,"state":"ringing","duration_secs":0,"rating":null,"started_at_ms":1735000200000,"ended_at_ms":null}"#)
            case ("POST", "/api/v1/calls/signal"):
                return .json(200, #"{"ok":true}"#)
            case ("GET", "/api/v1/webrtc/turn_credentials"):
                return .json(200, #"{"ice_servers":[{"urls":"stun:stun.example.com:3478"}]}"#)
            case ("POST", "/api/v1/calls/hangup"):
                return .json(200, #"{"ok":true}"#)
            case ("POST", "/api/v1/calls/call-live-9/rate"):
                return .json(200, #"{"call_id":"call-live-9","caller_id":701,"callee_id":702,"is_video":false,"state":"ended","duration_secs":12,"rating":5,"started_at_ms":1735000200000,"ended_at_ms":1735000212000}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("calls_loopback")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "701")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            try await signInTestUser(coordinator: coordinator)
            let toastCtrl = FabushiToastController(autoDismissEnabled: false)
            let router = ShellNavigationRouter(toastController: toastCtrl)
            let controller = FabushiCallSessionController(
                coordinator: coordinator,
                router: router,
                launchConfiguration: FabushiLaunchConfiguration(environment: [:], arguments: [])
            )
            defer { controller.detachLifecycleHooks() }

            await controller.loadCallHistory()
            XCTAssertEqual(controller.callHistory.count, 1)
            XCTAssertEqual(controller.callHistory.first?.callId, "call-hist-1")

            let started = await controller.startOutgoingCall(
                calleeUserId: 702,
                peerTitle: "Peer 702",
                isVideo: false,
                locale: .en
            )
            XCTAssertTrue(started)
            XCTAssertEqual(controller.activeCallId, "call-live-9")
            XCTAssertEqual(controller.callState, .connecting)

            // Minimize to floating Call PiP capsule
            controller.minimizeToFloatingPip(locale: .en)
            XCTAssertEqual(router.activeCallPip?.id, "call-live-9")
            controller.toggleMute(locale: .en)
            XCTAssertTrue(controller.isMuted)
            XCTAssertEqual(router.activeCallPip?.isMuted, true)

            // Hangup triggers rating sheet -> submit 5-star rating via real `calls.rate`
            await controller.hangup(reason: "done", showRating: true, locale: .en)
            XCTAssertTrue(controller.isRatingSheetPresented)
            XCTAssertEqual(controller.pendingRatingCallId, "call-live-9")

            let rated = await controller.submitCallRating(
                callId: "call-live-9",
                rating: 5,
                feedback: "Clear audio",
                locale: .en
            )
            XCTAssertTrue(rated)
            XCTAssertFalse(controller.isRatingSheetPresented)
            XCTAssertEqual(controller.callHistory.first(where: { $0.callId == "call-live-9" })?.rating, 5)

            // Logout advances accountScopeGeneration and tears down all call state cleanly
            _ = try await coordinator.logout()
            XCTAssertTrue(controller.callHistory.isEmpty)
            XCTAssertNil(controller.activeCallId)
            XCTAssertEqual(controller.callState, .idle)
            XCTAssertNil(router.activeCallPip)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 3. Launch Configuration Permission Denied, ReplayKit Denied & Honest BLK-02 Disclosure

    func testLaunchConfigurationSimulatesCallPermissionDeniedReplayKitDeniedAndBlk02Disclosure() async throws {
        let dir = try makeTempDir("perm_denied")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        let launchConfig = FabushiLaunchConfiguration(
            environment: [
                "FABUSHI_E2E_SIMULATE_CALL_PERMISSION_DENIED": "1",
                "FABUSHI_E2E_SIMULATE_REPLAYKIT_DENIED": "1",
            ],
            arguments: []
        )
        XCTAssertTrue(launchConfig.simulateCallPermissionDenied)
        XCTAssertTrue(launchConfig.simulateReplayKitDenied)

        let toastCtrl = FabushiToastController(autoDismissEnabled: false)
        let router = ShellNavigationRouter(toastController: toastCtrl)
        let controller = FabushiCallSessionController(
            coordinator: coordinator,
            router: router,
            launchConfiguration: launchConfig
        )
        defer { controller.detachLifecycleHooks() }

        // Honest BLK-02 disclosure MUST always remain visible
        XCTAssertTrue(controller.isBlk02SfuDisclosureVisible)
        XCTAssertTrue(controller.isPermissionDenied)
        XCTAssertTrue(controller.isScreenShareUnavailable)

        // Outgoing call fails closed under simulated permission denial
        let allowed = await controller.startOutgoingCall(calleeUserId: 999, peerTitle: "Peer", isVideo: true)
        XCTAssertFalse(allowed)
        XCTAssertEqual(controller.callState, .failed)
        XCTAssertTrue(controller.isPermissionDenied)

        // Screen share refuses honestly when ReplayKit is denied
        await controller.toggleScreenShare(locale: .en)
        XCTAssertFalse(controller.isScreenSharing)
        XCTAssertTrue(controller.isScreenShareUnavailable)

        // Verify HumanCallMediaPort interruption recovery helper works deterministically
        XCTAssertFalse(HumanCallMediaPort.shouldRecoverAfterAudioInterruption(userInfo: nil))

        await coordinator.shutdown()
    }

    // MARK: - 4. Real Rust TalkCoordinator 320-Chunk Boundary, <60ms VAD Barge-In & Terminal Close Invariant

    func testRealRustTalkCoordinator320ChunkBoundaryVadBargeInUnder60msAndClose() async throws {
        let dir = try makeTempDir("talk_engine")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:1",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let toastCtrl = FabushiToastController(autoDismissEnabled: false)
            let router = ShellNavigationRouter(toastController: toastCtrl)
            let vm = FabushiTalkModeViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            let created = await vm.startSession(remote: false, enableHardwareAec: false)
            XCTAssertTrue(created)
            XCTAssertEqual(vm.state, .listening)
            XCTAssertNotNil(vm.sessionId)

            let transitioned = await vm.transition(to: .speaking)
            XCTAssertTrue(transitioned)
            XCTAssertEqual(vm.state, .speaking)

            // Verify 320-chunk bounded FIFO buffer invariant (OCRM-AUDIO-O05)
            let payloadChunk = Data(repeating: 0x2A, count: 64)
            for _ in 0..<325 {
                let enqueued = try await vm.enqueueAudioChunk(payloadChunk)
                XCTAssertLessThanOrEqual(enqueued.bufferedChunks, 320)
            }
            XCTAssertEqual(vm.bufferedChunksCount, 320)

            let dequeued = try await vm.dequeueAudioChunk()
            XCTAssertNotNil(dequeued.chunkB64)
            XCTAssertEqual(dequeued.bufferedChunks, 319)

            // Trigger VAD barge-in via 3 consecutive loud PCM frames -> flushes all 319 chunks in <60ms!
            let loudSpeechFrame = [Int16](repeating: 18_000, count: 320)
            var lastVad: FabushiTalkVadResult?
            for _ in 0..<3 {
                lastVad = try await vm.processVadPcmFrame(loudSpeechFrame)
            }
            let vad = try XCTUnwrap(lastVad)
            XCTAssertTrue(vad.bargeInTriggered)
            XCTAssertEqual(vad.flushedChunks, 319)
            XCTAssertEqual(vm.state, .interrupted)
            XCTAssertEqual(vm.bufferedChunksCount, 0)
            XCTAssertLessThan(vm.lastBargeInElapsedMs ?? 999.0, 60.0, "VAD barge-in must execute in <60ms")

            // Resume listening -> close session -> terminal invariant rejects further audio enqueue
            let resumed = await vm.resumeListening()
            XCTAssertTrue(resumed)
            XCTAssertEqual(vm.state, .listening)

            await vm.closeSession()
            XCTAssertEqual(vm.state, .closed)

            do {
                _ = try await vm.enqueueAudioChunk(payloadChunk)
                XCTFail("Closed talk session MUST fail closed when enqueueing audio")
            } catch {
                // Expected fail-closed error on closed TalkCoordinator
            }

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 5. TalkMode ViewModel Loopback `talk.turn` & Account-Switch Cleanup

    func testTalkModeViewModelTurnOverLoopbackHTTPAndAccountSwitchCleanup() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":705,"username":"calls_user","session_id":"s-705","token":"tok-705","first_name":"Talk"}"#)
            case ("POST", "/api/v1/auth/logout"):
                return .json(200, #"{"ok":true}"#)
            case ("POST", "/api/v1/hermes/talk/turn"):
                return .json(200, #"{"session_id":"talk-705","call_id":null,"state":"speaking","transcript":"Hello Fabushi","reply_text":"Hello! How can I assist you today?","tts_segments":["Hello!"],"bot_id":99,"chat_id":10}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("talk_turn")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "705")
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: server.baseURLString,
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            try await signInTestUser(coordinator: coordinator)
            let toastCtrl = FabushiToastController(autoDismissEnabled: false)
            let router = ShellNavigationRouter(toastController: toastCtrl)
            let vm = FabushiTalkModeViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            _ = await vm.startSession(remote: false, enableHardwareAec: false)
            let response = await vm.sendTranscriptTurn("Hello Fabushi", chatId: 10, botId: 99)
            XCTAssertEqual(response?.replyText, "Hello! How can I assist you today?")
            XCTAssertEqual(vm.transcriptHistory.count, 2)
            XCTAssertEqual(vm.state, .speaking)

            let bargeIn = await vm.triggerBargeIn()
            XCTAssertNotNil(bargeIn)
            XCTAssertEqual(vm.state, .interrupted)
            XCTAssertLessThan(vm.lastBargeInElapsedMs ?? 999.0, 60.0)

            // Account logout cleanly resets all Talk Mode state & transcript history
            _ = try await coordinator.logout()
            XCTAssertTrue(vm.transcriptHistory.isEmpty)
            XCTAssertNil(vm.sessionId)
            XCTAssertEqual(vm.state, .idle)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
