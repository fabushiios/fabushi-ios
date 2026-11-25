import Foundation
import os
import SwiftUI
import XCTest
@testable import Fabushi

@MainActor
final class FabushiMessagingM3aStateAndSyncTests: XCTestCase, @unchecked Sendable {

    // MARK: - Test Harness Helpers (Real FabushiCoordinator + Real Rust Host + Real POSIX Loopback)

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
            try? keychain.deleteAccountKey(accountId: "2002")
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: cacheDirectory)
        }
    }

    private func makeHarness(
        authenticate: Bool = true,
        userId: UInt64 = 1001,
        username: String = "alice_m3a",
        handler: @escaping @Sendable (M2bLoopbackHTTPTestServer.Request) -> M2bLoopbackHTTPTestServer.Response
    ) async throws -> Harness {
        let uuid = UUID().uuidString
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m3a_host_\(uuid)", isDirectory: true)
        let cacheDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m3a_cache_\(uuid)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)

        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m3a.\(uuid)",
            account: "m3a-test"
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
            if authenticate {
                _ = try await coordinator.setAuthSession(
                    FabushiSessionAuthParams(
                        userId: userId,
                        username: username,
                        sessionId: "sess-\(userId)",
                        token: "tok-\(userId)"
                    )
                )
            }
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

    private static func makeChatDetailJSON(
        chatId: Int64,
        title: String,
        kind: String = "group",
        chatType: Int = 1,
        isGroup: Bool = true,
        isBot: Bool = false
    ) -> [String: Any] {
        [
            "chat_id": chatId,
            "title": title,
            "kind": kind,
            "chat_type": chatType,
            "creator_id": 1001,
            "is_group": isGroup,
            "is_channel": false,
            "is_bot": isBot,
            "is_forum": false,
            "is_archived": false,
            "is_pinned": false,
            "is_muted": false,
            "pts": 1,
            "last_message_id": 0,
            "last_message_date_ms": 1_730_000_000_000,
            "unread_count": 0,
            "read_inbox_max_id": 0,
            "read_outbox_max_id": 0,
            "member_count": isGroup ? 3 : 2,
        ]
    }

    // MARK: - 1. Bilingual Localization Parity & Forbidden-String Guards

    func testBilingualMessagingStringsParityAndForbiddenGuards() throws {
        let hanRegex = try NSRegularExpression(pattern: "\\p{Han}")
        let asciiLetterRegex = try NSRegularExpression(pattern: "[A-Za-z]")
        let forbiddenSampleTitles = [
            "Saved Messages",
            "Fabushi Community",
            "Sample Chat",
            "Demo Bot Chat",
            "Chief of Staff",
        ]

        XCTAssertGreaterThanOrEqual(FabushiMessagingStrings.Key.allCases.count, 100)
        XCTAssertEqual(FabushiMessagingStrings.localized(.personalNotes, locale: .en), "Personal Notes")
        XCTAssertEqual(FabushiMessagingStrings.localized(.personalNotes, locale: .zhHans), "收藏夹")

        for key in FabushiMessagingStrings.Key.allCases {
            let en = FabushiMessagingStrings.localized(key, locale: .en)
            let zh = FabushiMessagingStrings.localized(key, locale: .zhHans)

            XCTAssertFalse(en.isEmpty, "Empty en translation for \(key.rawValue)")
            XCTAssertFalse(zh.isEmpty, "Empty zh-Hans translation for \(key.rawValue)")
            XCTAssertNotEqual(en, key.rawValue, "en fell back to raw key \(key.rawValue)")
            XCTAssertNotEqual(zh, key.rawValue, "zh-Hans fell back to raw key \(key.rawValue)")
            XCTAssertNotEqual(en, zh, "en and zh-Hans must differ for \(key.rawValue)")

            let enRange = NSRange(en.startIndex..<en.endIndex, in: en)
            XCTAssertNotNil(
                asciiLetterRegex.firstMatch(in: en, range: enRange),
                "en must contain ASCII letters for \(key.rawValue): \(en)"
            )
            XCTAssertNil(
                hanRegex.firstMatch(in: en, range: enRange),
                "en must NOT contain Han characters for \(key.rawValue): \(en)"
            )

            let zhRange = NSRange(zh.startIndex..<zh.endIndex, in: zh)
            XCTAssertNotNil(
                hanRegex.firstMatch(in: zh, range: zhRange),
                "zh-Hans must contain Han characters for \(key.rawValue): \(zh)"
            )

            for value in [en, zh] {
                for prefix in FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes {
                    XCTAssertFalse(
                        value.lowercased().contains(prefix),
                        "Forbidden raw key prefix '\(prefix)' leaked in '\(value)' for \(key.rawValue)"
                    )
                }
                for sample in forbiddenSampleTitles {
                    XCTAssertFalse(
                        value.localizedCaseInsensitiveContains(sample),
                        "Forbidden sample title '\(sample)' leaked in '\(value)' for \(key.rawValue)"
                    )
                }
            }
        }
    }

    // MARK: - 2. Dual Wire Schema Decoding (TRAP #2, TRAP #3), Dynamic Catalog (`D10-09`) & `BLK-08`

    func testDualSchemaAndRawPayloadEventDecodingAndBLK08() throws {
        // Online ChatSummary vs Online ChatDetails (`members_count`) vs Local ChatDetailDto (`member_count`)
        let summaryJSON = #"{"chat_id":101,"title":"RustBot, FinBot","chat_type":1,"unread_count":3,"last_message":"Ready","is_group":true,"is_bot":true,"updated_at":1730000100000}"#
        let summary = try JSONDecoder().decode(FabushiMessagingChat.self, from: Data(summaryJSON.utf8))
        XCTAssertEqual(summary.chatId, 101)
        XCTAssertTrue(summary.isGroup)
        XCTAssertFalse(summary.isBot, "D14-04: Group chats must NEVER decode as 1:1 isBot == true")
        XCTAssertEqual(summary.lastMessagePreview, "Ready")

        let detailsJSON = #"{"chat_id":102,"title":"Engineering","chat_type":1,"members_count":42,"pts":99}"#
        let details = try JSONDecoder().decode(FabushiMessagingChat.self, from: Data(detailsJSON.utf8))
        XCTAssertEqual(details.memberCount, 42)
        XCTAssertEqual(details.pts, 99)

        let localDtoJSON = #"{"chat_id":103,"title":"OpsBot","kind":"bot","chat_type":3,"creator_id":1,"is_group":false,"is_channel":false,"is_bot":true,"is_forum":false,"is_archived":false,"is_pinned":true,"is_muted":false,"pts":12,"last_message_id":7,"last_message_date_ms":1730000200000,"unread_count":0,"read_inbox_max_id":7,"read_outbox_max_id":7,"member_count":2}"#
        let localDto = try JSONDecoder().decode(FabushiMessagingChat.self, from: Data(localDtoJSON.utf8))
        XCTAssertTrue(localDto.isBot)
        XCTAssertTrue(localDto.isPinned)
        XCTAssertEqual(localDto.memberCount, 2)

        // Online HistoricalMessage (`message_id`, `chosen`, `reply_to_info`)
        let historicalJSON = #"{"message_id":501,"chat_id":101,"sender_id":9,"text":"Online hello","date":1730000300000,"is_outgoing":false,"reply_to_info":{"reply_to_msg_id":500,"quote_text":"quoted snippet"},"reactions":[{"emoji":"👍","count":2,"chosen":true}]}"#
        let historical = try JSONDecoder().decode(FabushiMessageRecord.self, from: Data(historicalJSON.utf8))
        XCTAssertEqual(historical.messageId, 501)
        XCTAssertEqual(historical.text, "Online hello")
        XCTAssertEqual(historical.replyTo?.replyToMsgId, 500)
        XCTAssertEqual(historical.replyTo?.quoteText, "quoted snippet")
        XCTAssertEqual(historical.reactions.first?.isChosenByMe, true)

        // Live WS MessageItem (`id`, nested `content.data.text`, RFC3339 `date`, `is_chosen_by_me`) — TRAP #2
        let wsItemJSON = #"{"id":502,"chat_id":101,"sender_id":10,"date":"2026-11-25T14:00:00Z","content":{"type":"Text","data":{"text":"Nested WS frame","entities":[{"offset":0,"length":6,"entity_type":{"type":"Bold"}}]}},"reply_to":{"reply_to_msg_id":501,"quote_text":"Online hello"},"reactions":[{"emoji":"🔥","count":4,"is_chosen_by_me":true}]}"#
        let wsDecoded = try XCTUnwrap(FabushiMessageRecord.decodeFromDomainEventPayload(wsItemJSON))
        XCTAssertEqual(wsDecoded.messageId, 502)
        XCTAssertEqual(wsDecoded.text, "Nested WS frame")
        XCTAssertGreaterThan(wsDecoded.dateMs, 0)
        XCTAssertEqual(wsDecoded.replyTo?.replyToMsgId, 501)
        XCTAssertEqual(wsDecoded.replyTo?.quoteText, "Online hello")
        XCTAssertEqual(wsDecoded.reactions.first?.isChosenByMe, true)
        XCTAssertEqual(wsDecoded.entities.first?.entityType, .bold)

        // Reaction event payload decoding (`is_chosen_by_me`) — TRAP #2
        let reactionPayload = #"{"chat_id":101,"message_id":502,"user_id":1001,"reactions":[{"emoji":"❤️","count":1,"is_chosen_by_me":true}]}"#
        let parsedReactions = try XCTUnwrap(FabushiMessageRecord.decodeReactionsFromEventPayload(reactionPayload))
        XCTAssertEqual(parsedReactions.reactions.first?.emoji, "❤️")
        XCTAssertEqual(parsedReactions.reactions.first?.isChosenByMe, true)

        // Dynamic `FabushiAiModelCatalog` (`D10-09`, ZERO hardcoded `"gpt-5.4"`)
        let magpieRaw: FabushiJSONValue = .object([
            "active_provider": .string("deepseek"),
            "active_model": .string("deepseek-reasoner"),
            "providers": .array([
                .object([
                    "id": .string("deepseek"),
                    "enabled": .bool(true),
                    "models": .array([.string("deepseek-chat"), .string("deepseek-reasoner")]),
                ]),
            ]),
            "routing_groups": .array([.object(["name": .string("fast-tier")])]),
        ])
        let catalog = FabushiAiModelCatalog.fromMagpieStateJSON(magpieRaw)
        XCTAssertTrue(catalog.hasConfiguredProvider)
        XCTAssertEqual(catalog.activeModel, "deepseek-reasoner")
        XCTAssertFalse(catalog.models.contains { $0.modelId == "gpt-5.4" })
        XCTAssertEqual(FabushiAiModelCatalog.normalizePreferredModel("default"), "")
        XCTAssertEqual(FabushiAiModelCatalog.normalizePreferredModel("默认"), "")
        XCTAssertEqual(FabushiAiModelCatalog.normalizePreferredModel("hermes-local-v1"), "")
        XCTAssertTrue(catalog.isMismatchedOverride("legacy-removed-model"))
        XCTAssertFalse(catalog.isMismatchedOverride("deepseek-chat"))
        XCTAssertFalse(catalog.isMismatchedOverride("default"))

        // `BLK-08`: Exactly 8 backed scopes, ZERO Community scope
        XCTAssertEqual(FabushiMessagingCapabilityBoundaries.communityFoundationStatus, .upstreamFoundation)
        XCTAssertFalse(FabushiMessagingCapabilityBoundaries.communitySearchSupported)
        XCTAssertEqual(FabushiSearchScope.allCases.count, 8)
        XCTAssertFalse(FabushiSearchScope.allCases.contains { $0.rawValue.lowercased().contains("community") })
    }

    // MARK: - 3. `D13-01` & `D13-02` Frame-0 Cache, 100-Message Cap, Offline Preservation & Account/Server Purgers

    func testD13_01AndD13_02Frame0HydrationCapOfflinePreservationAndPurgers() async throws {
        let modeLock = OSAllocatedUnfairLock<Int>(initialState: 503)

        let harness = try await makeHarness { req in
            let mode = modeLock.withLock { $0 }
            if req.path == "/api/v1/chats" {
                if mode == 503 {
                    return .json(503, #"{"error":"upstream maintenance"}"#)
                }
                if mode == 401 {
                    return .json(401, #"{"error":"token expired"}"#)
                }
                return .jsonObject(200, [])
            }
            if req.path == "/api/v1/auth/logout" {
                return .json(200, #"{"ok":true}"#)
            }
            return .json(200, "[]")
        }

        do {
            let serverURL = harness.coordinator.settingsStore.activeServerURL

            // Verify SHA-256 key isolation across server URLs & accounts
            let keyA = FabushiMessagingCacheStore.cacheKey(serverURL: "https://a.example.com/", accountId: "1001")
            let keyANorm = FabushiMessagingCacheStore.cacheKey(serverURL: "https://a.example.com", accountId: "1001")
            let keyB = FabushiMessagingCacheStore.cacheKey(serverURL: "https://b.example.com", accountId: "1001")
            let keyC = FabushiMessagingCacheStore.cacheKey(serverURL: "https://a.example.com", accountId: "2002")
            XCTAssertEqual(keyA, keyANorm)
            XCTAssertNotEqual(keyA, keyB)
            XCTAssertNotEqual(keyA, keyC)
            let fileURL = harness.cacheStore.snapshotFileURL(serverURL: serverURL, accountId: "1001")
            XCTAssertFalse(fileURL.lastPathComponent.contains("1001"), "Account ID must never appear in plaintext filename")

            // Save snapshot with 135 messages -> must enforce <= 100 messages/chat cap (`D13-01`)
            let oversizedHistory = (1...135).map { idx in
                FabushiMessageRecord(
                    messageId: Int64(idx),
                    chatId: 300,
                    senderId: 1001,
                    text: "Message \(idx)",
                    dateMs: 1_730_000_000_000 + Int64(idx * 1000)
                )
            }
            let seededSnapshot = FabushiSyncSnapshot(
                lastActiveChatId: 300,
                chats: [
                    FabushiMessagingChat(chatId: 300, title: "Cached Thread", isPinned: true, lastMessagePreview: "Message 135", lastMessageDateMs: 1_730_000_135_000),
                ],
                folders: [FabushiFolder(id: 1, title: "Work", includeGroups: true)],
                contacts: [FabushiContact(userId: 50, firstName: "Bob", username: "bob")],
                messagesByChat: [300: oversizedHistory],
                draftsByChat: [300: FabushiDraftRecord(chatId: 300, text: "Saved draft")]
            )
            XCTAssertTrue(harness.cacheStore.saveSnapshot(seededSnapshot, serverURL: serverURL, accountId: "1001"))

            // Synchronous Frame-0 hydration before any network round-trip
            XCTAssertTrue(harness.store.hydrateFrame0())
            XCTAssertEqual(harness.store.chats.count, 1)
            XCTAssertEqual(harness.store.chats.first?.title, "Cached Thread")
            XCTAssertEqual(harness.store.lastActiveChatId, 300)
            XCTAssertEqual(harness.store.messagesByChat[300]?.count, 100)
            XCTAssertEqual(harness.store.messagesByChat[300]?.first?.messageId, 36)
            XCTAssertEqual(harness.store.messagesByChat[300]?.last?.messageId, 135)
            XCTAssertEqual(harness.store.unsentDraftTextByChat[300], "Saved draft")

            // `D13-02`: Transient HTTP 503 preserves hydrated workspace offline without wiping disk cache
            await harness.store.refreshWorkspace(trigger: .initialAppear)
            XCTAssertTrue(harness.store.offlinePreservedMode)
            XCTAssertEqual(harness.store.chats.count, 1)
            XCTAssertNotNil(harness.cacheStore.loadSnapshot(serverURL: serverURL, accountId: "1001"))

            // `D13-02`: HTTP 401 wipes disk cache & clears in-memory workspace
            modeLock.withLock { $0 = 401 }
            await harness.store.refreshWorkspace(trigger: .manual)
            XCTAssertTrue(harness.store.chats.isEmpty)
            XCTAssertNil(harness.cacheStore.loadSnapshot(serverURL: serverURL, accountId: "1001"))

            // Re-seed two accounts and verify logout purges departing account only, then server switch purges all!
            XCTAssertTrue(harness.cacheStore.saveSnapshot(seededSnapshot, serverURL: serverURL, accountId: "1001"))
            XCTAssertTrue(harness.cacheStore.saveSnapshot(seededSnapshot, serverURL: serverURL, accountId: "2002"))
            XCTAssertEqual(harness.cacheStore.snapshotFileCount(), 2)

            _ = try await harness.coordinator.logout()
            XCTAssertNil(harness.cacheStore.loadSnapshot(serverURL: serverURL, accountId: "1001"))
            XCTAssertNotNil(harness.cacheStore.loadSnapshot(serverURL: serverURL, accountId: "2002"))

            _ = try await harness.coordinator.setServerURL("http://127.0.0.1:9")
            XCTAssertEqual(harness.cacheStore.snapshotFileCount(), 0)

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 4. `D10-06` Compose Epoch Cancellation, Unrelated Chat Delete Isolation & `D14-05` WS Subscribe

    func testD10_06ComposeEpochCancellationDeleteIsolationAndD14_05WsSubscribe() async throws {
        let shouldDelayCreate = OSAllocatedUnfairLock<Bool>(initialState: true)

        let harness = try await makeHarness { req in
            if req.method == "POST" && req.path == "/api/v1/chats/create" {
                if shouldDelayCreate.withLock({ $0 }) {
                    Thread.sleep(forTimeInterval: 0.14)
                }
                return .jsonObject(
                    200,
                    Self.makeChatDetailJSON(chatId: 7701, title: "Release War Room", kind: "group", chatType: 1, isGroup: true, isBot: false)
                )
            }
            if req.method == "DELETE" && req.path == "/api/v1/chats/500" {
                return .json(200, #"{"deleted":true}"#)
            }
            return .json(200, "[]")
        }

        do {
            let human = FabushiComposeRecipient(userId: 201, displayName: "Bob", username: "bob", isBot: false)
            let bot = FabushiComposeRecipient(userId: 202, displayName: "RustBot", username: "rustbot", isBot: true)

            // Case A: Late `chats.create` returns AFTER `cancelComposeMode()` -> dropped, never hijacks active chat!
            harness.store.beginComposeMode()
            harness.store.toggleComposeRecipient(human)
            harness.store.toggleComposeRecipient(bot)
            let initialEpoch = harness.store.composeEpoch

            let createTask = Task { @MainActor in
                await harness.store.createComposeConversation(router: harness.router, locale: .en)
            }
            try await Task.sleep(nanoseconds: 25_000_000)
            harness.store.cancelComposeMode()
            XCTAssertGreaterThan(harness.store.composeEpoch, initialEpoch)
            XCTAssertFalse(harness.store.isComposeActive)

            let cancelledResult = await createTask.value
            XCTAssertNil(cancelledResult, "D10-06: Cancelled compose MUST drop late chats.create completion")
            XCTAssertNil(harness.store.activeChatId)
            XCTAssertNotEqual(harness.router.activeRoute, .chats(chatId: 7701, messageId: nil))

            // Case B: Deleting an unrelated chat while compose is active preserves compose chips & group title!
            harness.store.upsertChat(FabushiMessagingChat(chatId: 500, title: "Old Chat"))
            harness.store.beginComposeMode()
            harness.store.toggleComposeRecipient(human)
            harness.store.toggleComposeRecipient(bot)
            harness.store.composeGroupTitle = "Release War Room"

            try await harness.store.deleteChat(chatId: 500)
            XCTAssertTrue(harness.store.isComposeActive, "D10-06: Unrelated chat delete must preserve compose mode")
            XCTAssertEqual(harness.store.selectedComposeRecipients.count, 2)
            XCTAssertEqual(harness.store.composeGroupTitle, "Release War Room")
            XCTAssertFalse(harness.store.chats.contains { $0.chatId == 500 })

            // Case C: Successful group creation keeps `isGroup == true`, `isBot == false`, AND subscribes WS (`D14-05`)!
            shouldDelayCreate.withLock { $0 = false }
            let created = await harness.store.createComposeConversation(router: harness.router, locale: .en)
            let createdChat = try XCTUnwrap(created)
            XCTAssertEqual(createdChat.chatId, 7701)
            XCTAssertTrue(createdChat.isGroup)
            XCTAssertFalse(createdChat.isBot)
            XCTAssertTrue(harness.store.subscribedWsChatIds.contains(7701), "D14-05: Group creation must subscribe WS")
            XCTAssertEqual(harness.router.activeRoute, .chats(chatId: 7701, messageId: nil))

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 5. `D14-03` `sendDurable` Reply+Quote Wire Proof & `D10-07` Draft Preservation / Failure Restoration

    func testD14_03SendDurableForwardsReplyAndQuoteAndD10_07DraftGuards() async throws {
        let failSend = OSAllocatedUnfairLock<Bool>(initialState: false)

        let harness = try await makeHarness { req in
            if req.method == "POST" && req.path == "/api/v1/messages/send" {
                if failSend.withLock({ $0 }) {
                    return .json(400, #"{"error":"invalid message payload"}"#)
                }
                return .jsonObject(200, [
                    "message_id": 8801,
                    "id": 8801,
                    "pts": 12,
                    "pts_count": 1,
                    "duplicate": false,
                    "date_ms": 1_730_000_500_000,
                    "date": "2026-11-25T15:00:00Z",
                    "sender_id": 1001,
                    "chat_id": 600,
                    "text": "Ack",
                    "status": "sent",
                ])
            }
            if req.path == "/api/v1/chats/600/draft" {
                if req.method == "DELETE" {
                    return .json(200, #"{"cleared":true}"#)
                }
                return .jsonObject(200, [
                    "chat_id": 600,
                    "text": "saved",
                    "entities": [],
                    "date_ms": 1_730_000_500_000,
                ])
            }
            return .json(200, "[]")
        }

        do {
            harness.store.upsertChat(FabushiMessagingChat(chatId: 600, title: "Bot Group", isGroup: true))
            harness.router.navigate(to: .chats(chatId: 600, messageId: nil), coordinator: harness.coordinator)

            // 1. `D14-03`: Outgoing reply via `sendComposerMessage` uses `messages.sendDurable`
            //    forwarding BOTH `reply_to` AND `quote_text` on the real HTTP wire!
            let replyTarget = FabushiReplyTarget(
                messageId: 42,
                senderName: "RustBot",
                quoteText: "Check Swift 6 strict concurrency"
            )
            let sentResp = await harness.store.sendComposerMessage(
                chatId: 600,
                text: "On it!",
                replyTarget: replyTarget,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertEqual(sentResp?.messageId, 8801)

            let sendRequests = harness.server.requests(method: "POST", pathPrefix: "/api/v1/messages/send")
            let firstBody = try XCTUnwrap(sendRequests.first?.body)
            let wireJSON = try XCTUnwrap(try JSONSerialization.jsonObject(with: firstBody) as? [String: Any])
            XCTAssertEqual(wireJSON["reply_to"] as? Int, 42, "D14-03: wire payload MUST carry reply_to")
            XCTAssertEqual(
                wireJSON["quote_text"] as? String,
                "Check Swift 6 strict concurrency",
                "D14-03: wire payload MUST carry quote_text (TRAP #1)"
            )

            // 2. `D10-07`: Non-composer sends (e.g. `/start`, stickers, reply keyboard) MUST preserve unsent draft!
            await harness.store.updateComposerDraft(chatId: 600, text: "Unsent composer draft", persistRemote: false)
            _ = try await harness.store.sendNonComposerMessage(chatId: 600, text: "/start")
            XCTAssertEqual(
                harness.store.unsentDraftTextByChat[600],
                "Unsent composer draft",
                "D10-07: Non-composer send MUST preserve unsent composer text"
            )

            // 3. `D10-07`: Permanent send failure restores unsent text + shows chat-scoped `"Message send failed"` toast!
            failSend.withLock { $0 = true }
            let failedResp = await harness.store.sendComposerMessage(
                chatId: 600,
                text: "Critical unsent draft",
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertNil(failedResp)
            XCTAssertEqual(
                harness.store.unsentDraftTextByChat[600],
                "Critical unsent draft",
                "D10-07: Permanent send failure MUST restore unsent draft"
            )
            XCTAssertEqual(harness.router.toastController.windowBanner?.message, "Message send failed")
            XCTAssertEqual(harness.router.toastController.windowBanner?.scopeChatId, 600)
            XCTAssertEqual(harness.router.toastController.windowBanner?.isError, true)

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 6. `D14-04` Group Bot Activity Non-Corruption, Auto-Clear & Stale-Generation Event Fence

    func testD14_04GroupBotActivityNonCorruptionAndStaleEventFence() async throws {
        let harness = try await makeHarness { _ in .json(200, "[]") }

        do {
            // Group chat with `"Bot"` in title MUST remain `isGroup == true, isBot == false`
            let groupChat = FabushiMessagingChat(
                chatId: 900,
                title: "RustBot, FinBot",
                kind: "group",
                chatType: 1,
                isGroup: true,
                isBot: false
            )
            let dmBotChat = FabushiMessagingChat(
                chatId: 901,
                title: "RustBot",
                kind: "bot",
                chatType: 3,
                isGroup: false,
                isBot: true
            )
            harness.store.upsertChat(groupChat)
            harness.store.upsertChat(dmBotChat)

            XCTAssertTrue(harness.store.knownBotIds.contains(901))
            XCTAssertFalse(harness.store.knownBotIds.contains(900))
            XCTAssertTrue(harness.store.canDisplayBotActivity(for: 900))
            XCTAssertTrue(harness.store.canDisplayBotActivity(for: 901))

            let gen = harness.coordinator.accountScopeGeneration

            // Stale generation event MUST be dropped
            let staleEvent = FabushiDomainEvent.parse(
                seq: 1,
                generation: gen - 1,
                timestampMs: 1_730_000_000_000,
                type: "botActivityChanged",
                rawPayloadJSON: #"{"chat_id":900,"bot_user_id":901,"state":"thinking","detail":"Stale"}"#
            )
            XCTAssertFalse(harness.store.ingestDomainEvent(staleEvent))
            XCTAssertNil(harness.store.activeBotActivityByChat[900])

            // Current generation group `botActivityChanged` displays thinking WITHOUT flipping `isBot` or polluting `knownBotIds`!
            let activeEvent = FabushiDomainEvent.parse(
                seq: 2,
                generation: gen,
                timestampMs: 1_730_000_001_000,
                type: "botActivityChanged",
                rawPayloadJSON: #"{"chat_id":900,"bot_user_id":901,"state":"thinking","detail":"Analyzing diff..."}"#
            )
            XCTAssertTrue(harness.store.ingestDomainEvent(activeEvent))
            XCTAssertEqual(harness.store.activeBotActivityByChat[900]?.isActive, true)
            XCTAssertEqual(harness.store.chats.first(where: { $0.chatId == 900 })?.isBot, false)
            XCTAssertFalse(harness.store.knownBotIds.contains(900))

            // Incoming live WS `newMessage` with nested Rust `MessageItem` JSON decodes text & auto-clears `"thinking"`!
            let replyEvent = FabushiDomainEvent.parse(
                seq: 3,
                generation: gen,
                timestampMs: 1_730_000_002_000,
                type: "newMessage",
                rawPayloadJSON: #"{"id":999,"chat_id":900,"sender_id":901,"date":"2026-11-25T15:05:00Z","content":{"type":"Text","data":{"text":"Diff looks clean!","entities":[]}}}"#
            )
            XCTAssertTrue(harness.store.ingestDomainEvent(replyEvent))
            XCTAssertNil(harness.store.activeBotActivityByChat[900], "D14-04: Incoming message must auto-clear thinking state")
            XCTAssertEqual(harness.store.messagesByChat[900]?.last?.text, "Diff looks clean!")

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 7. `D10-08` Authoritative Blocklist & TTL Post-RPC Toast + Error Rollback

    func testD10_08AuthoritativeBlockAndTTLRollbackAndPostRPCToast() async throws {
        let shouldFail = OSAllocatedUnfairLock<Bool>(initialState: true)

        let harness = try await makeHarness { req in
            let fail = shouldFail.withLock { $0 }
            if req.path == "/api/v1/users/555/block" {
                if fail { return .json(500, #"{"error":"server unavailable"}"#) }
                if req.method == "POST" {
                    return .jsonObject(200, [
                        "user_id": 555,
                        "first_name": "Mallory",
                        "username": "mallory",
                        "blocked_at_ms": 1_730_000_600_000,
                    ])
                }
                return .json(200, #"{"unblocked":true}"#)
            }
            if req.path == "/api/v1/chats/100/ttl" {
                if fail { return .json(500, #"{"error":"ttl write failed"}"#) }
                return .json(200, #"{"ttl_seconds":86400}"#)
            }
            return .json(200, "[]")
        }

        do {
            harness.store.upsertChat(FabushiMessagingChat(chatId: 100, title: "Alice", ttlSeconds: 0))
            harness.router.navigate(to: .chats(chatId: 100, messageId: nil), coordinator: harness.coordinator)

            // HTTP 500 -> rollback + error toast (NO success toast!)
            let blockFailed = await harness.store.setUserBlocked(userId: 555, blocked: true, router: harness.router, locale: .en)
            XCTAssertFalse(blockFailed)
            XCTAssertFalse(harness.store.isUserBlocked(555))
            XCTAssertEqual(harness.router.toastController.windowBanner?.isError, true)
            harness.router.toastController.clearAll()

            let ttlFailed = await harness.store.setChatTTL(chatId: 100, ttlSeconds: 86_400, router: harness.router, isCompact: false, locale: .en)
            XCTAssertFalse(ttlFailed)
            XCTAssertEqual(harness.store.chats.first(where: { $0.chatId == 100 })?.ttlSeconds, 0)
            XCTAssertEqual(harness.router.toastController.windowBanner?.isError, true)
            harness.router.toastController.clearAll()

            // HTTP 200 -> authoritative update + post-RPC confirmation toast!
            shouldFail.withLock { $0 = false }
            let blockOk = await harness.store.setUserBlocked(userId: 555, blocked: true, router: harness.router, locale: .en)
            XCTAssertTrue(blockOk)
            XCTAssertTrue(harness.store.isUserBlocked(555))
            XCTAssertEqual(harness.router.toastController.windowBanner?.message, "User blocked")
            XCTAssertEqual(harness.router.toastController.windowBanner?.isError, false)
            harness.router.toastController.clearAll()

            let ttlOk = await harness.store.setChatTTL(chatId: 100, ttlSeconds: 86_400, router: harness.router, isCompact: false, locale: .en)
            XCTAssertTrue(ttlOk)
            XCTAssertEqual(harness.store.chats.first(where: { $0.chatId == 100 })?.ttlSeconds, 86_400)
            XCTAssertEqual(harness.router.toastController.windowBanner?.message, "Auto-delete timer updated")
            XCTAssertEqual(harness.router.toastController.windowBanner?.isError, false)

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 8. `D13-04` `<3s` WS Dedup & `D13-05` Bounded Top-8 Prefetcher + Low Power / Constrained Guards

    func testD13_04WsConnectDedupAndD13_05BoundedPrefetcherGuards() async throws {
        let harness = try await makeHarness { req in
            if req.path == "/api/v1/chats" {
                let rows: [[String: Any]] = (1...12).map { idx in
                    [
                        "chat_id": idx,
                        "title": "Chat \(idx)",
                        "chat_type": 0,
                        "unread_count": 0,
                        "updated_at": 1_730_000_000_000 + (idx * 1000),
                    ]
                }
                return .jsonObject(200, rows)
            }
            if req.path.hasPrefix("/api/v1/chats/") && req.path.hasSuffix("/messages") {
                return .jsonObject(200, [
                    [
                        "message_id": 1,
                        "sender_id": 2,
                        "text": "Hello",
                        "date": 1_730_000_000_000,
                    ],
                ])
            }
            return .json(200, "[]")
        }

        do {
            // Suppress prefetch during Low Power Mode
            harness.store.isLowPowerModeProvider = { true }
            await harness.store.refreshWorkspace(trigger: .initialAppear)
            try await Task.sleep(nanoseconds: 60_000_000)
            XCTAssertEqual(harness.store.prefetchExecutedChatIds.count, 0, "D13-05: Low Power Mode must skip prefetch")

            // `<3s` WS initial-connect dedup MUST suppress duplicate refresh burst!
            let dispatchesBefore = harness.store.wsRefreshDispatchCount
            await harness.store.refreshWorkspace(trigger: .wsInitialConnect)
            XCTAssertEqual(harness.store.wsRefreshDispatchCount, dispatchesBefore, "D13-04: <3s WS initial connect must dedup")

            // Reconnect MUST bypass dedup!
            harness.store.isLowPowerModeProvider = { false }
            harness.store.isConstrainedNetworkProvider = { true }
            await harness.store.refreshWorkspace(trigger: .wsReconnect)
            XCTAssertEqual(harness.store.wsRefreshDispatchCount, dispatchesBefore + 1, "D13-04: WS reconnect must refresh")
            try await Task.sleep(nanoseconds: 60_000_000)
            XCTAssertEqual(harness.store.prefetchExecutedChatIds.count, 0, "D13-05: Constrained network must skip prefetch")

            // Normal power + unconstrained network -> prefetch capped at <= 8 chats (pinned first)!
            harness.store.isConstrainedNetworkProvider = { false }
            harness.store.upsertChat(FabushiMessagingChat(chatId: 1, title: "Pinned 1", isPinned: true, lastMessageDateMs: 1_730_000_001_000))
            harness.store.scheduleBackgroundHistoryPrefetch(capturedGeneration: harness.coordinator.accountScopeGeneration)
            try await Task.sleep(nanoseconds: 250_000_000)

            XCTAssertGreaterThan(harness.store.prefetchExecutedChatIds.count, 0)
            XCTAssertLessThanOrEqual(harness.store.prefetchExecutedChatIds.count, FabushiMessagingStore.maxPrefetchDialogs)
            XCTAssertTrue(harness.store.prefetchExecutedChatIds.contains(1), "Pinned chat must be prioritized in top-8 prefetch")

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 9. Feature #12 PTS/Outbox, Feature #18 Search (`BLK-08`), Contacts/Prestart Bot & SwiftUI Smoke

    func testSyncPtsOutboxUnifiedSearchContactsPrestartBotAndViewsSmoke() async throws {
        let harness = try await makeHarness { req in
            if req.path == "/api/v1/sync/get_difference" {
                return .jsonObject(200, [
                    "chat_id": 400,
                    "new_messages": [
                        [
                            "id": 701,
                            "chat_id": 400,
                            "peer_id": ["Chat": 400],
                            "to_peer": ["Chat": 400],
                            "sender_id": 88,
                            "flags": [
                                "is_out": false,
                                "is_pinned": false,
                                "is_silent": false,
                                "is_edited": false,
                                "is_scheduled": false,
                                "media_unread": false,
                                "has_reactions": false,
                                "no_forwards": false,
                                "is_forwarded": false,
                            ],
                            "date": "2026-11-25T15:10:00Z",
                            "content": ["type": "Text", "data": ["text": "Recovered gap message", "entities": []]],
                        ],
                    ],
                    "other_updates": [],
                    "pts": 5,
                    "current_pts": 5,
                    "has_more": false,
                    "snapshot_required": false,
                ])
            }
            if req.path == "/api/v1/search/messages" {
                return .jsonObject(200, [
                    "query": "rust",
                    "total": 1,
                    "hits": [
                        [
                            "chat_id": 400,
                            "message_id": 701,
                            "sender_id": 88,
                            "date_ms": 1_730_000_700_000,
                            "text": "Rust search hit",
                            "snippet_html": "<mark>Rust</mark> search hit",
                        ],
                    ],
                    "results": [],
                ])
            }
            if req.path == "/api/v1/search" {
                return .jsonObject(200, [
                    "query": "rust",
                    "category": "all",
                    "total": 2,
                    "hits": [],
                    "results": [
                        ["chat_id": 999, "title": "Leak Community", "snippet": "blocked", "category": "community"],
                    ],
                    "apps": [
                        [
                            "app_id": "app-1",
                            "bot_user_id": 901,
                            "bot_username": "rustbot",
                            "bot_display_name": "RustBot",
                            "title": "Rust Runner",
                            "short_name": "runner",
                        ],
                    ],
                    "bots": [
                        ["bot_user_id": 901, "owner_user_id": 1, "username": "rustbot", "display_name": "RustBot"],
                    ],
                ])
            }
            if req.path == "/api/v1/contacts" {
                return .jsonObject(200, [
                    ["user_id": 501, "first_name": "Ada", "username": "ada", "is_bot": false, "is_mutual": true, "chat_id": 501],
                ])
            }
            if req.path == "/api/v1/peers/resolve" {
                return .jsonObject(200, [
                    "user_id": 502,
                    "username": "linus",
                    "first_name": "Linus",
                    "is_bot": false,
                    "status": "online",
                ])
            }
            if req.path == "/api/v1/bots/profile/rustbot" {
                return .jsonObject(200, [
                    "bot": [
                        "bot_user_id": 901,
                        "owner_user_id": 1,
                        "username": "rustbot",
                        "display_name": "RustBot",
                        "description": "Helps audit Swift and Rust",
                    ],
                    "has_started_chat": false,
                    "allows_write_to_pm": false,
                    "is_blocked": false,
                    "is_muted": false,
                ])
            }
            if req.path == "/api/v1/bots/901/allow-send-message" {
                return .jsonObject(200, [
                    "ok": true,
                    "chat_id": 901,
                    "service_text": "Bot started",
                ])
            }
            if req.path == "/api/v1/messages/send" {
                return .jsonObject(200, [
                    "message_id": 9011,
                    "id": 9011,
                    "pts": 2,
                    "pts_count": 1,
                    "duplicate": false,
                    "date_ms": 1_730_000_800_000,
                    "date": "2026-11-25T15:12:00Z",
                    "sender_id": 1001,
                    "chat_id": 901,
                    "text": "/start",
                    "status": "sent",
                ])
            }
            return .json(200, "[]")
        }

        do {
            // Feature #12: Real Rust PTS contiguous -> duplicate -> gap_detected + sync.getDifference
            _ = try await harness.store.client.setPts(chatId: 400, pts: 1)
            let dummyEvent: FabushiJSONValue = .object([
                "type": .string("ReadHistory"),
                "data": .object([
                    "chat_id": .int(400),
                    "max_id": .int(1),
                    "is_outbox": .bool(false),
                    "unread_count": .int(0),
                ]),
            ])
            let evalContiguous = try await harness.store.client.evaluatePtsUpdate(chatId: 400, pts: 2, ptsCount: 1, event: dummyEvent)
            XCTAssertEqual(evalContiguous.outcome, "contiguous")

            let evalDuplicate = try await harness.store.client.evaluatePtsUpdate(chatId: 400, pts: 2, ptsCount: 1, event: dummyEvent)
            XCTAssertEqual(evalDuplicate.outcome, "duplicate")

            let evalGap = try await harness.store.client.evaluatePtsUpdate(chatId: 400, pts: 5, ptsCount: 1, event: dummyEvent)
            XCTAssertEqual(evalGap.outcome, "gap_detected")

            await harness.store.syncDifferenceForChat(400)
            XCTAssertEqual(harness.store.ptsByChat[400], 5)
            XCTAssertEqual(harness.store.messagesByChat[400]?.last?.text, "Recovered gap message")

            // Feature #12: Real Rust SQLite WAL Outbox enqueue, list, retryNow & dismiss
            let outboxNonce = harness.store.nextClientNonce()
            let enqueued = try await harness.store.client.enqueueOutbox(
                FabushiRichSendMessageRequest(chatId: 400, text: "Offline queued note", clientNonce: outboxNonce)
            )
            XCTAssertEqual(enqueued.clientNonce, outboxNonce)
            await harness.store.refreshOutbox()
            XCTAssertTrue(harness.store.outboxEntries.contains { $0.clientNonce == outboxNonce })
            _ = await harness.store.retryAllOutbox()
            harness.store.dismissOutboxEntry(clientNonce: outboxNonce)
            XCTAssertFalse(harness.store.visibleOutboxEntries.contains { $0.clientNonce == outboxNonce })

            // Feature #18: Tantivy + SQLite FTS5 CJK local index + global catalog (zero Community leakage per `BLK-08`)
            try await harness.store.client.indexLocalSearchMessage(
                messageId: 702,
                chatId: 400,
                senderId: 1001,
                timestampMs: 1_730_000_750_000,
                body: "法布施搜索测试 rust"
            )
            await harness.store.performSearch(query: "rust", scope: .all)
            XCTAssertTrue(harness.store.searchResults.contains { $0.kind == .message && $0.messageId == 701 })
            XCTAssertTrue(harness.store.searchResults.contains { $0.kind == .bot && $0.title == "RustBot" })
            XCTAssertTrue(harness.store.searchResults.contains { $0.kind == .miniApp && $0.miniAppId == "app-1" })
            XCTAssertFalse(harness.store.searchResults.contains { $0.title.contains("Community") }, "BLK-08: Must never emit Community hits")

            await harness.store.performSearch(query: "法布施搜索测试", scope: .messages)
            XCTAssertTrue(harness.store.searchResults.contains { $0.messageId == 702 && $0.isOfflineLocalHit })

            // Feature #11: Contacts, `@username` resolve & Prestart Bot hero flow preserving drafts (`D10-07`)
            await harness.store.refreshContactsAndBlocklist()
            XCTAssertEqual(harness.store.contacts.first?.username, "ada")
            let resolved = await harness.store.resolveUsernamePeer("@linus")
            XCTAssertEqual(resolved?.userId, 502)

            await harness.store.updateComposerDraft(chatId: 901, text: "Keep my draft", persistRemote: false)
            let botProfile = try XCTUnwrap(await harness.store.loadBotPublicProfile(target: "rustbot"))
            XCTAssertFalse(botProfile.hasStartedChat)
            let startedChatId = await harness.store.startPrestartBot(
                botId: botProfile.bot.botId,
                targetKey: "rustbot",
                router: harness.router,
                locale: .en
            )
            XCTAssertEqual(startedChatId, 901)
            XCTAssertEqual(harness.store.unsentDraftTextByChat[901], "Keep my draft", "Prestart /start must preserve unsent draft")

            // SwiftUI View instantiation smoke across all 4 themes & both locales
            for preset in FabushiThemePreset.allCases {
                for locale in FabushiLocale.allCases {
                    let theme = FabushiTheme(preset: preset, locale: locale)
                    _ = FabushiChatListView(coordinator: harness.coordinator, router: harness.router, store: harness.store)
                        .environment(theme)
                    _ = FabushiMultiSelectComposeView(coordinator: harness.coordinator, router: harness.router, store: harness.store)
                        .environment(theme)
                    _ = FabushiUniversalSearchView(coordinator: harness.coordinator, router: harness.router, store: harness.store)
                        .environment(theme)
                    _ = FabushiContactsView(coordinator: harness.coordinator, router: harness.router, store: harness.store)
                        .environment(theme)
                    _ = FabushiPrestartBotHeroView(
                        profile: botProfile,
                        store: harness.store,
                        coordinator: harness.coordinator,
                        router: harness.router
                    )
                    .environment(theme)
                    _ = FabushiDialogListSkeletonView().environment(theme)
                    _ = FabushiCollapsedRailSkeletonView().environment(theme)
                    _ = FabushiTranscriptSkeletonView().environment(theme)
                    _ = FabushiStageSyncingPlaceholderView().environment(theme)
                }
            }

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }
}
