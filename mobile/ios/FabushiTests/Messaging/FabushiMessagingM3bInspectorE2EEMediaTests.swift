import Foundation
import os
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif
import XCTest
@testable import Fabushi

@MainActor
final class FabushiMessagingM3bInspectorE2EEMediaTests: XCTestCase, @unchecked Sendable {

    // MARK: - Isolated Real-Host + Loopback Test Harness (Zero Mocks)

    private struct Harness {
        let directory: URL
        let cacheDirectory: URL
        let keychain: FabushiKeychainStore
        let server: M2bLoopbackHTTPTestServer
        let coordinator: FabushiCoordinator
        let cacheStore: FabushiMessagingCacheStore
        let store: FabushiMessagingStore
        let router: ShellNavigationRouter

        @MainActor
        func tearDown() async {
            store.shutdown()
            await coordinator.shutdown()
            server.stop()
            try? keychain.deletePassphrase()
            try? keychain.deleteAccountKey(accountId: "1001")
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: cacheDirectory)
        }
    }

    private func makeHarness(
        handler: @escaping @Sendable (M2bLoopbackHTTPTestServer.Request) -> M2bLoopbackHTTPTestServer.Response
    ) async throws -> Harness {
        let uuid = UUID().uuidString
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m3b_iem_host_\(uuid)", isDirectory: true)
        let cacheDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m3b_iem_cache_\(uuid)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)

        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m3b.iem.\(uuid)",
            account: "m3b-iem-test"
        )
        let server = try M2bLoopbackHTTPTestServer(handler: handler)
        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: keychain)
        let cacheStore = FabushiMessagingCacheStore(baseDirectory: cacheDir)
        let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))

        do {
            try await coordinator.start(
                appDataDirectory: dir,
                serverURL: server.baseURLString,
                testMode: true,
                startNetworkMonitor: false
            )
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 1001,
                    username: "alice_m3b_iem",
                    sessionId: "sess-iem",
                    token: "tok-iem"
                )
            )
            let store = FabushiMessagingStore(coordinator: coordinator, cacheStore: cacheStore)
            store.prefetchDelayNanoseconds = 0
            return Harness(
                directory: dir,
                cacheDirectory: cacheDir,
                keychain: keychain,
                server: server,
                coordinator: coordinator,
                cacheStore: cacheStore,
                store: store,
                router: router
            )
        } catch {
            await coordinator.shutdown()
            server.stop()
            try? keychain.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
            try? FileManager.default.removeItem(at: cacheDir)
            throw error
        }
    }

    // MARK: - 1. `D10-09` Dynamic AI Model Catalog & `D13-04` 2-Wave Parallel Inspector Fetch

    func testD1009DynamicAiModelCatalogFollowSettingsMismatchAndD1304TwoWaveInspector() async throws {
        let patchedPreferredModel = OSAllocatedUnfairLock<String?>(initialState: nil)

        let harness = try await makeHarness { req in
            switch (req.method, req.path) {
            case ("GET", "/api/v1/chats/301"):
                return .jsonObject(200, [
                    "chat_id": 301,
                    "title": "Research Assistant",
                    "about": "Helps inspect documents",
                    "chat_type": "bot",
                    "creator_id": 1001,
                    "is_forum": false,
                    "members_count": 2,
                    "pts": 15,
                ])
            case ("GET", "/api/v1/chats/301/members"):
                return .jsonObject(200, [[
                    "user_id": 1001,
                    "first_name": "Alice",
                    "role": "owner",
                    "admin_rights": 0,
                    "joined_at_ms": 1_730_000_000_000,
                ]])
            case ("GET", "/api/v1/chats/301/invites"):
                return .jsonObject(200, [])
            case ("GET", "/api/v1/chats/301/pins"):
                return .jsonObject(200, [])
            case ("GET", "/api/v1/chats/301/draft"):
                return .json(200, "null")
            case ("GET", "/api/v1/chats/301/statistics"):
                return .jsonObject(200, [
                    "chat_id": 301,
                    "title": "Research Assistant",
                    "member_count": 2,
                    "total_messages": 14,
                    "messages_last_7d": 5,
                    "total_reactions": 2,
                    "active_posters_7d": 2,
                    "growth_rate_pct": 0.0,
                    "daily_series": [],
                ])
            case ("GET", "/api/v1/chats/301/send_as"):
                return .jsonObject(200, [[
                    "peer_id": 1001,
                    "title": "Alice",
                    "kind": "user",
                    "is_default": true,
                ]])
            case ("GET", "/api/v1/hermes/magpie/state"):
                return .jsonObject(200, [
                    "active_provider": "deepseek",
                    "active_model": "deepseek-chat",
                    "providers": [[
                        "id": "deepseek",
                        "enabled": true,
                        "default_model": "deepseek-chat",
                        "models": ["deepseek-chat", "deepseek-reasoner"],
                    ]],
                    "routing_groups": [["name": "fast-lane"]],
                ])
            case ("GET", "/api/v1/fabushi-bots/301"):
                return .jsonObject(200, [
                    "bot_id": 301,
                    "display_name": "Research Assistant",
                    "username": "research_bot",
                    "preferred_model": "retired-legacy-model",
                    "thinking_level": "medium",
                    "approval_mode": "ask",
                    "updated_at_ms": 1_730_000_000_000,
                ])
            case ("PATCH", "/api/v1/fabushi-bots/301"):
                if let obj = try? JSONSerialization.jsonObject(with: Data(req.bodyString.utf8)) as? [String: Any] {
                    patchedPreferredModel.withLock { $0 = obj["preferred_model"] as? String }
                }
                return .jsonObject(200, [
                    "bot_id": 301,
                    "display_name": "Research Assistant",
                    "username": "research_bot",
                    "preferred_model": "",
                    "updated_at_ms": 1_730_000_100_000,
                ])
            case ("GET", "/api/v1/bots/301/computer"):
                return .jsonObject(200, [
                    "bot_id": 301,
                    "bound_node_id": "node-studio-1",
                    "persona_prompt": "Be concise",
                    "preferred_model": "retired-legacy-model",
                    "approval_mode": "ask",
                    "allow_cross_device_control": false,
                    "allow_cloud_self_config": false,
                    "updated_at_ms": 1_730_000_000_000,
                ])
            case ("PUT", "/api/v1/bots/301/computer"):
                return .jsonObject(200, [
                    "bot_id": 301,
                    "bound_node_id": "node-studio-1",
                    "persona_prompt": "Be concise",
                    "preferred_model": "",
                    "approval_mode": "ask",
                    "allow_cross_device_control": false,
                    "allow_cloud_self_config": false,
                    "updated_at_ms": 1_730_000_100_000,
                ])
            case ("GET", "/api/v1/computers"):
                return .jsonObject(200, [[
                    "node_id": "node-studio-1",
                    "kind": "desktop",
                    "display_name": "Mac Studio",
                    "online": true,
                    "bound_bot_ids": [301],
                ]])
            case ("GET", "/api/v1/bots/profile/301"), ("GET", "/api/v1/bots/profile/research_bot"):
                return .jsonObject(200, [
                    "bot": [
                        "bot_user_id": 301,
                        "username": "research_bot",
                        "display_name": "Research Assistant",
                        "description": "Research bot",
                    ],
                    "has_started_chat": true,
                    "allows_write_to_pm": true,
                ])
            default:
                return .jsonObject(200, [:])
            }
        }

        do {
            // `D13-04` 2-Wave Parallel Inspector Load
            let snapshot = await FabushiInspectorLoader.load(
                target: .bot(botId: 301, chatId: 301),
                store: harness.store
            )
            XCTAssertGreaterThanOrEqual(snapshot.wave1CompletedCount, 6)
            XCTAssertGreaterThanOrEqual(snapshot.wave2CompletedCount, 3)
            XCTAssertEqual(snapshot.chat?.title, "Research Assistant")
            XCTAssertEqual(snapshot.members.count, 1)
            XCTAssertEqual(snapshot.computers.first?.nodeId, "node-studio-1")

            // `D10-09` Dynamic AI Model Catalog strictly from live `magpie.state`, ZERO hardcoded `"gpt-5.4"`
            XCTAssertTrue(snapshot.modelCatalog.hasConfiguredProvider)
            XCTAssertEqual(snapshot.modelCatalog.activeProvider, "deepseek")
            XCTAssertEqual(snapshot.modelCatalog.activeModel, "deepseek-chat")
            XCTAssertFalse(snapshot.modelCatalog.models.contains(where: { $0.modelId.contains("gpt-5.4") }))

            // Mismatch Warning Banner triggers for retired override not in live Settings catalog
            var modelState = FabushiInspectorBotModelState(
                preferredModel: snapshot.botComputerProfile?.preferredModel,
                catalog: snapshot.modelCatalog,
                profileUpdatedAtMs: snapshot.botComputerProfile?.updatedAtMs ?? 0
            )
            XCTAssertTrue(modelState.showsMismatchWarning)
            XCTAssertFalse(modelState.isFollowingSettings)

            // Follow Settings clears `preferred_model` to `""` and disables mismatch warning
            modelState.selectFollowSettings()
            XCTAssertTrue(modelState.isFollowingSettings)
            XCTAssertFalse(modelState.showsMismatchWarning)
            XCTAssertEqual(modelState.effectiveActiveModelLabel, "deepseek / deepseek-chat")

            // Sentinel normalization checks
            for sentinel in ["", "default", "默认", "默认模型", "auto", "hermes-local-v1", "local_hermes/hermes-local-v1"] {
                XCTAssertEqual(FabushiAiModelCatalog.normalizePreferredModel(sentinel), "")
            }

            // Patching `preferred_model: ""` persists over loopback
            let patched = try await harness.store.client.patchBot(botId: 301, preferredModel: "")
            XCTAssertEqual(FabushiAiModelCatalog.normalizePreferredModel(patched.preferredModel), "")
            XCTAssertEqual(patchedPreferredModel.withLock { $0 }, "")

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 2. Feature #19 Real Pure-Rust E2EE Crypto & Fail-Closed Server Prekey Boundary

    func testFeature19RealRustE2EEIdentitySafetyFingerprintRatchetSenderKeyAndFailClosed() async throws {
        let plaintextSendCalled = OSAllocatedUnfairLock<Bool>(initialState: false)

        let harness = try await makeHarness { req in
            if req.path.contains("/api/v1/e2ee/") {
                return .jsonObject(404, ["error": "peer_prekeys_not_published"])
            }
            if req.path == "/api/v1/messages/send" {
                plaintextSendCalled.withLock { $0 = true }
            }
            return .jsonObject(404, ["error": "not_found"])
        }

        do {
            // 1. Real pure-Rust `e2ee.identity` & symmetric 4-emoji `e2ee.safetyFingerprint`
            let id = try await harness.store.client.e2eeIdentity()
            XCTAssertFalse(id.x25519PubB64.isEmpty)
            XCTAssertFalse(id.ed25519PubB64.isEmpty)
            XCTAssertFalse(id.deviceId.isEmpty)

            let remotePubB64 = Data(repeating: 0x42, count: 32).base64EncodedString()
            let emojisAB = try await harness.store.client.e2eeSafetyFingerprint(
                localIdentityKeyB64: id.x25519PubB64,
                remoteIdentityKeyB64: remotePubB64
            )
            let emojisBA = try await harness.store.client.e2eeSafetyFingerprint(
                localIdentityKeyB64: remotePubB64,
                remoteIdentityKeyB64: id.x25519PubB64
            )
            XCTAssertFalse(emojisAB.isEmpty)
            XCTAssertEqual(emojisAB, emojisBA, "Safety number 4-emoji fingerprint must be symmetric")

            let safetyInfo = try await harness.store.loadE2EESafetyInfo(
                chatId: 401,
                peerTitle: "Bob",
                remoteIdentityKeyB64: remotePubB64
            )
            XCTAssertEqual(safetyInfo.safetyEmojis, emojisAB)
            #if canImport(UIKit)
            XCTAssertNotNil(FabushiQRCodeRenderer.renderQRCodeImage(from: safetyInfo.qrPayload, scale: 6))
            #endif
            let clipPayload = FabushiSecureClipboard.makeSensitivePayload(safetyInfo.localIdentityKeyB64)
            XCTAssertTrue(clipPayload.localOnly)

            // 2. Real pure-Rust Double Ratchet encrypt/decrypt roundtrip
            _ = try await harness.store.client.e2eeRatchetInitPair(
                aliceSessionId: "alice-dr-m3b",
                bobSessionId: "bob-dr-m3b"
            )
            let encrypted = try await harness.store.client.e2eeRatchetEncrypt(
                sessionId: "alice-dr-m3b",
                plaintext: "Top secret Double Ratchet message"
            )
            let decrypted = try await harness.store.client.e2eeRatchetDecrypt(
                sessionId: "bob-dr-m3b",
                header: encrypted.header,
                ciphertextB64: encrypted.ciphertextB64
            )
            XCTAssertEqual(decrypted, "Top secret Double Ratchet message")

            // 3. Real pure-Rust Group Sender Key roundtrip
            let dist = try await harness.store.client.e2eeSenderKeyCreate(
                keyId: "sender-alice-m3b",
                chatId: 401,
                senderUserId: 1001,
                senderDeviceId: id.deviceId
            )
            try await harness.store.client.e2eeSenderKeyImportDistribution(
                keyId: "receiver-bob-m3b",
                distribution: dist
            )
            let groupCiphertext = try await harness.store.client.e2eeSenderKeyEncrypt(
                keyId: "sender-alice-m3b",
                plaintext: "Group SenderKey secret"
            )
            let groupPlaintext = try await harness.store.client.e2eeSenderKeyDecrypt(
                keyId: "receiver-bob-m3b",
                ciphertext: groupCiphertext
            )
            XCTAssertEqual(groupPlaintext, "Group SenderKey secret")

            // 4. Fail-closed remote E2EE prekey boundary: MUST error & NEVER fall back to plaintext!
            harness.router.navigate(to: .chats(chatId: 401, messageId: nil), coordinator: harness.coordinator)
            let result = await harness.store.sendEncryptedMessageFailClosed(
                chatId: 401,
                recipientUserId: 2002,
                isGroup: false,
                plaintext: "Must never downgrade",
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            if case .success = result {
                XCTFail("Expected fail-closed failure when peer prekeys return 404")
            }
            XCTAssertFalse(plaintextSendCalled.withLock { $0 }, "E2EE send MUST NEVER silently downgrade to plaintext")
            XCTAssertEqual(
                harness.router.toastController.activeWindowToast?.message,
                FabushiMessagingStrings.localized(.e2eePeerNotEnrolledNotice, locale: .en)
            )

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 3. Feature #20 5-Bit Waveform, Photo Editor, Media Range/CAS, Stickers/GIFs, Stories & Instant View

    func testFeature20WaveformPhotoEditorMediaStickersGifsStoriesAndInstantView() async throws {
        let samplePayload = Data("0123456789ABCDEF0123456789ABCDEF".utf8)

        let harness = try await makeHarness { req in
            if req.method == "GET" && req.path == "/api/v1/media/range-file-1" {
                let slice = samplePayload.subdata(in: 0..<10)
                return M2bLoopbackHTTPTestServer.Response(
                    statusCode: 206,
                    statusText: "Partial Content",
                    contentType: "application/octet-stream",
                    body: slice
                )
            }
            if req.method == "GET" && req.path == "/api/v1/stickers/sets" {
                return .jsonObject(200, [[
                    "set_id": "pack-1",
                    "short_name": "fabushi_pack",
                    "title": "Fabushi Pack",
                    "count": 1,
                    "is_installed": false,
                    "stickers": [["file_id": "stk-1", "emoji": "🚀"]],
                ]])
            }
            if req.method == "POST" && req.path == "/api/v1/stickers/sets/pack-1" {
                return .jsonObject(200, ["set_id": "pack-1", "installed": true])
            }
            if req.method == "GET" && req.path == "/api/v1/gifs/search" {
                return .jsonObject(200, [[
                    "id": "gif-1",
                    "title": "Rocket Launch",
                    "file_id": "gif-file-1",
                    "url": "https://example.com/rocket.gif",
                    "preview_url": "https://example.com/rocket-preview.gif",
                    "width": 320,
                    "height": 240,
                    "mime_type": "image/gif",
                ]])
            }
            if req.method == "GET" && req.path == "/api/v1/stories" {
                return .jsonObject(200, [[
                    "story_id": "st-1",
                    "user_id": 1001,
                    "first_name": "Alice",
                    "media_type": "photo",
                    "caption": "Morning Coffee",
                    "privacy": "everyone",
                    "views_count": 3,
                    "is_viewed": false,
                    "created_at_ms": 1_730_000_000_000,
                    "expires_at_ms": 1_730_086_400_000,
                ]])
            }
            if req.method == "POST" && req.path == "/api/v1/stories" {
                return .jsonObject(200, [
                    "story_id": "st-2",
                    "user_id": 1001,
                    "first_name": "Alice",
                    "media_type": "photo",
                    "caption": "Evening Sunset",
                    "privacy": "everyone",
                    "views_count": 0,
                    "is_viewed": false,
                    "created_at_ms": 1_730_000_010_000,
                    "expires_at_ms": 1_730_086_410_000,
                ])
            }
            if req.method == "DELETE" && req.path == "/api/v1/stories/st-2" {
                return .jsonObject(200, ["deleted": true])
            }
            return .jsonObject(200, [:])
        }

        do {
            // 1. Real Rust `media.encodeWaveform` / `decodeWaveform` + Swift `WaveformCodec5Bit` + Voice Player
            let barsInput: [UInt8] = (0..<100).map { UInt8($0 % 32) }
            let rustEncoded = try await harness.store.client.encodeWaveform(bars: barsInput)
            XCTAssertEqual(rustEncoded.bytes.count, WaveformCodec5Bit.packedByteCount)
            let rustDecoded = try await harness.store.client.decodeWaveform(waveformB64: rustEncoded.base64, targetBars: 100)
            XCTAssertEqual(rustDecoded.count, 100)

            let normalized = FabushiVoicePlayerController.normalizedWaveformBars(from: rustEncoded.bytes, targetCount: 40)
            XCTAssertEqual(normalized.count, 40)
            let player = FabushiVoicePlayerController()
            XCTAssertEqual(player.playbackRate, 1.0)
            player.cyclePlaybackRate()
            XCTAssertEqual(player.playbackRate, 1.5)
            player.cyclePlaybackRate()
            XCTAssertEqual(player.playbackRate, 2.0)

            // 2. Real HTTP 206 `media.downloadRange` + Local CAS roundtrip
            let rangeResp = try await harness.store.client.downloadMediaRange(
                fileId: "range-file-1",
                start: 0,
                endInclusive: 9
            )
            XCTAssertEqual(rangeResp.status, 206)
            XCTAssertEqual(rangeResp.sizeBytes, 10)

            try await harness.store.client.casPut(fileId: "cas-m3b-1", data: samplePayload, mimeType: "application/octet-stream")
            let casBack = try await harness.store.client.casGet(fileId: "cas-m3b-1")
            XCTAssertEqual(casBack, samplePayload)

            // 3. Real Photo Editor rotation, crop & annotation flattening
            #if canImport(UIKit)
            let renderer = UIGraphicsImageRenderer(size: CGSize(width: 120, height: 60))
            let sourceImage = renderer.image { ctx in
                UIColor.systemBlue.setFill()
                ctx.fill(CGRect(x: 0, y: 0, width: 120, height: 60))
            }
            let edited = try XCTUnwrap(
                FabushiPhotoEditorEngine.renderEditedImage(
                    source: sourceImage,
                    quarterTurnsClockwise: 1,
                    cropAspect: .square,
                    strokes: [FabushiAnnotationStroke(points: [CGPoint(x: 5, y: 5), CGPoint(x: 40, y: 40)])]
                )
            )
            XCTAssertEqual(Int(edited.image.size.width), Int(edited.image.size.height))
            XCTAssertFalse(edited.jpegData.isEmpty)
            #endif

            // 4. Stickers, GIFs & Stories CRUD
            let sets = try await harness.store.client.listStickerSets()
            XCTAssertEqual(sets.first?.setId, "pack-1")
            try await harness.store.client.installStickerSet(setId: "pack-1", installed: true)
            let gifs = try await harness.store.client.searchGifs(query: "rocket")
            XCTAssertEqual(gifs.first?.id, "gif-1")

            await harness.store.refreshStories()
            XCTAssertEqual(harness.store.stories.first?.storyId, "st-1")
            let createdStory = await harness.store.createStory(caption: "Evening Sunset")
            XCTAssertEqual(createdStory?.storyId, "st-2")
            let deletedStory = await harness.store.deleteStory(storyId: "st-2")
            XCTAssertTrue(deletedStory)

            // 5. Instant View parser
            let ivDoc = FabushiInstantViewParser.parse(
                url: "https://docs.example.com/article",
                fallbackTitle: "Fallback",
                rawText: "# Apple HIG Guide\n\n> Crisp native typography.\n\n```swift\nlet x = 1\n```\n\nBody paragraph."
            )
            XCTAssertEqual(ivDoc.title, "Apple HIG Guide")
            XCTAssertEqual(ivDoc.siteName, "docs.example.com")
            XCTAssertGreaterThanOrEqual(ivDoc.blocks.count, 3)

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 4. Sealed `FabushiMessagingModule` (All 9 Factory Methods) & Full Bilingual Strings Parity

    func testSealedMessagingModuleAll9FactoryMethodsAndStringsParityNoHardcodedModels() async throws {
        // 1. Bilingual parity across 100% of `FabushiMessagingStrings.Key.allCases` + zero `"gpt-5.4"`
        let hanRegex = try NSRegularExpression(pattern: "\\p{Han}")
        let asciiLetterRegex = try NSRegularExpression(pattern: "[A-Za-z]")
        let forbiddenTitles = ["Saved Messages", "Fabushi Community", "Sample Chat", "Demo Bot Chat", "Chief of Staff", "gpt-5.4"]

        XCTAssertGreaterThanOrEqual(FabushiMessagingStrings.Key.allCases.count, 200)

        for key in FabushiMessagingStrings.Key.allCases {
            let en = FabushiMessagingStrings.localized(key, locale: .en)
            let zh = FabushiMessagingStrings.localized(key, locale: .zhHans)
            XCTAssertFalse(en.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, "Empty en for \(key)")
            XCTAssertFalse(zh.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, "Empty zh-Hans for \(key)")
            XCTAssertNotEqual(en, zh, "Identical en/zh for \(key)")

            let enRange = NSRange(en.startIndex..<en.endIndex, in: en)
            let zhRange = NSRange(zh.startIndex..<zh.endIndex, in: zh)
            XCTAssertNotNil(asciiLetterRegex.firstMatch(in: en, range: enRange), "en missing ASCII letters for \(key): \(en)")
            XCTAssertNil(hanRegex.firstMatch(in: en, range: enRange), "en contains Han for \(key): \(en)")
            XCTAssertNotNil(hanRegex.firstMatch(in: zh, range: zhRange), "zh-Hans missing Han for \(key): \(zh)")

            for prefix in FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes {
                XCTAssertFalse(en.lowercased().contains(prefix), "en leaks prefix \(prefix) for \(key)")
                XCTAssertFalse(zh.lowercased().contains(prefix), "zh leaks prefix \(prefix) for \(key)")
            }
            for forbidden in forbiddenTitles {
                XCTAssertFalse(en.localizedCaseInsensitiveContains(forbidden), "en contains forbidden '\(forbidden)' for \(key)")
                XCTAssertFalse(zh.localizedCaseInsensitiveContains(forbidden), "zh contains forbidden '\(forbidden)' for \(key)")
            }
        }

        // 2. Register sealed `FabushiMessagingModule` into an isolated registry and build all 9 views
        let harness = try await makeHarness { _ in .jsonObject(200, [:]) }
        do {
            let module = FabushiMessagingModule()
            let registry = FabushiFeatureModuleRegistry()
            registry.register(messaging: module)

            for preset in FabushiThemePreset.allCases {
                for locale in FabushiLocale.allCases {
                    let theme = FabushiTheme(preset: preset, locale: locale)
                    let views: [AnyView] = [
                        registry.messaging.makeChatSidebarView(coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeContactsView(coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeConversationStageView(chatId: 101, messageId: nil, coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeMultiSelectComposeView(coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeContextInspectorView(target: .chat(chatId: 101), coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeUniversalSearchView(coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeMediaViewerOverlay(coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeStoriesOverlay(coordinator: harness.coordinator, router: harness.router),
                        registry.messaging.makeInstantViewOverlay(coordinator: harness.coordinator, router: harness.router),
                    ]
                    XCTAssertEqual(views.count, 9)
                    _ = views.map { $0.environment(theme).environment(registry) }
                }
            }

            FabushiMessagingStoreRegistry.shared.removeStore(for: harness.coordinator)
            await harness.tearDown()
        } catch {
            FabushiMessagingStoreRegistry.shared.removeStore(for: harness.coordinator)
            await harness.tearDown()
            throw error
        }
    }
}
