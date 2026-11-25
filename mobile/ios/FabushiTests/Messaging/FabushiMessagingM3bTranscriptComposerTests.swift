import Foundation
import os
import SwiftUI
import XCTest
@testable import Fabushi

@MainActor
final class FabushiMessagingM3bTranscriptComposerTests: XCTestCase, @unchecked Sendable {

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
            try? keychain.deleteAccountKey(accountId: "2002")
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: cacheDirectory)
        }
    }

    private func makeHarness(
        handler: @escaping @Sendable (M2bLoopbackHTTPTestServer.Request) -> M2bLoopbackHTTPTestServer.Response
    ) async throws -> Harness {
        let uuid = UUID().uuidString
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m3b_tc_host_\(uuid)", isDirectory: true)
        let cacheDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m3b_tc_cache_\(uuid)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: cacheDir, withIntermediateDirectories: true)

        let keychain = FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m3b.tc.\(uuid)",
            account: "m3b-tc-test"
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
                    username: "alice_m3b",
                    sessionId: "sess-m3b",
                    token: "tok-m3b"
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

    // MARK: - 1. `D14-03` Reply + Quote Forwarding via `messages.sendDurable`

    func testD1403SendDurableForwardsBothReplyToAndQuoteTextOverLoopback() async throws {
        let capturedBody = OSAllocatedUnfairLock<String?>(initialState: nil)

        let harness = try await makeHarness { req in
            if req.method == "POST" && req.path == "/api/v1/messages/send" {
                capturedBody.withLock { $0 = req.bodyString }
                return .jsonObject(200, [
                    "message_id": 9001,
                    "id": 9001,
                    "pts": 42,
                    "pts_count": 1,
                    "duplicate": false,
                    "date_ms": 1_730_500_000_000,
                    "sender_id": 1001,
                    "chat_id": 501,
                    "text": "Following up on that quote",
                    "status": "sent",
                ])
            }
            if req.method == "DELETE" && req.path == "/api/v1/chats/501/draft" {
                return .jsonObject(200, ["cleared": true])
            }
            return .jsonObject(200, [:])
        }

        do {
            harness.store.upsertChat(FabushiMessagingChat(chatId: 501, title: "RustBot Team", kind: "group", isGroup: true, isBot: false))
            let reply = FabushiReplyTarget(
                messageId: 77,
                senderName: "RustBot",
                quoteText: "Cargo check passed with 0 warnings"
            )
            let response = await harness.store.sendComposerMessage(
                chatId: 501,
                text: "Following up on that quote",
                replyTarget: reply,
                router: harness.router,
                isCompact: false,
                locale: .en
            )

            XCTAssertEqual(response?.messageId, 9001)
            let rawWire = try XCTUnwrap(capturedBody.withLock { $0 })
            let json = try XCTUnwrap(
                try JSONSerialization.jsonObject(with: Data(rawWire.utf8)) as? [String: Any]
            )
            XCTAssertEqual(json["reply_to"] as? Int, 77)
            XCTAssertEqual(json["quote_text"] as? String, "Cargo check passed with 0 warnings")

            let storedMsg = try XCTUnwrap(harness.store.messagesByChat[501]?.first(where: { $0.messageId == 9001 }))
            XCTAssertEqual(storedMsg.replyTo?.replyToMsgId, 77)
            XCTAssertEqual(storedMsg.replyTo?.quoteText, "Cargo check passed with 0 warnings")

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 2. `D11-01` 50+ Slash Commands (`/distill` + `/distill-script`), UI Effects & `@` Context Badges

    func testD1101RealRustListCommandsIncludes50PlusAndDistillPlusUiEffectAndContextBadges() async throws {
        let harness = try await makeHarness { req in
            if req.method == "POST" && req.path == "/api/v1/hermes/command" {
                return .jsonObject(200, [
                    "handled": true,
                    "command": "/distill",
                    "session_id": "sess-distill",
                    "output": "Distilled checkout script",
                    "reply_text": "Created reusable browser script",
                    "inline_keyboard": [],
                    "effects": [[
                        "type": "distill_browser_script",
                        "app_id": "app-checkout-1",
                        "slug": "checkout-helper",
                        "name": "Checkout Helper",
                        "app_json": "{\"steps\":3}",
                    ]],
                ])
            }
            return .jsonObject(200, [:])
        }

        do {
            // 1. Real offline Rust `fabushi.listCommands`
            await harness.store.refreshSlashCommands()
            let commands = harness.store.slashCommands
            XCTAssertGreaterThanOrEqual(commands.count, 50, "Expected 50+ Hermes/Fabushi slash commands from Rust host")

            let categories = Set(commands.map { $0.category.lowercased() })
            XCTAssertGreaterThanOrEqual(categories.count, 7, "Expected >= 7 slash command categories")

            let distillCmd = try XCTUnwrap(
                commands.first(where: { $0.name == "distill" || $0.slashName == "/distill" }),
                "D11-01 requires /distill command in real Rust registry"
            )
            XCTAssertTrue(
                distillCmd.aliases.contains("distill-script"),
                "D11-01 requires distill-script alias on /distill"
            )

            // 2. Real Rust `fabushi.autocompleteCommands`
            let autocomplete = try await harness.store.client.autocompleteSlashCommands(prefix: "/distill")
            XCTAssertTrue(autocomplete.contains(where: { $0.slashName == "/distill" }))

            // 3. `D11-01` Execute `/distill` and verify `distill_browser_script` UI effect populates card
            let execResult = await harness.store.executeSlashCommand(
                "/distill checkout",
                chatId: 502,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertEqual(execResult?.handled, true)
            let distilled = try XCTUnwrap(harness.store.distilledScriptsByChat[502]?.first)
            XCTAssertEqual(distilled.appId, "app-checkout-1")
            XCTAssertEqual(distilled.slug, "checkout-helper")
            XCTAssertEqual(distilled.name, "Checkout Helper")

            // 4. `@file`, `@dir`, `@diff`, `@git:`, `@url` Context Badges formatting
            let badges = [
                FabushiContextBadge(kind: .file, label: "Store.swift", value: "Store.swift"),
                FabushiContextBadge(kind: .directory, label: "Presentation", value: "Presentation"),
                FabushiContextBadge(kind: .diff, label: "HEAD~1", value: "HEAD~1"),
                FabushiContextBadge(kind: .git, label: "main", value: "main"),
                FabushiContextBadge(kind: .url, label: "Docs", value: "https://example.com/spec"),
            ]
            let composed = FabushiSlashCommandCatalogHelper.composeMessageText(body: "Review these", badges: badges)
            XCTAssertTrue(composed.contains("@file Store.swift"))
            XCTAssertTrue(composed.contains("@dir Presentation"))
            XCTAssertTrue(composed.contains("@diff HEAD~1"))
            XCTAssertTrue(composed.contains("@git:main"))
            XCTAssertTrue(composed.contains("@url https://example.com/spec"))
            XCTAssertTrue(composed.hasSuffix("\nReview these"))

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 3. `D11-02` & `D10-08` Inline Keyboard Router (Browser Takeover, `computer:`, MiniApp, IV & Honest Fallback)

    func testD1102AndD1008InlineKeyboardActionsRouteCleanlyWithoutFakeSuccess() async throws {
        let harness = try await makeHarness { req in
            if req.method == "POST" && req.path == "/api/v1/hermes/command" {
                return .jsonObject(200, ["handled": false, "command": "", "output": "", "inline_keyboard": [], "effects": []])
            }
            return .jsonObject(200, [:])
        }

        do {
            harness.router.navigate(to: .chats(chatId: 601, messageId: nil), coordinator: harness.coordinator)

            // 1. `D11-02`: browser_takeover / browser_resume / browser_cancel
            for callback in ["browser_takeover:sid-42", "browser_resume:sid-42", "browser_cancel:sid-42"] {
                await harness.store.handleInlineKeyboardAction(
                    button: FabushiInlineKeyboardButton(text: "Browser", callbackData: callback),
                    chatId: 601,
                    messageId: 10,
                    router: harness.router,
                    isCompact: false,
                    locale: .en
                )
                XCTAssertEqual(harness.store.lastBrowserTakeoverAction, callback)
                XCTAssertEqual(
                    harness.router.toastController.activeWindowToast?.message,
                    FabushiMessagingStrings.localized(.browserTakeoverUnavailableToast, locale: .en)
                )
            }

            // 2. `D10-08`: `computer:<nodeId>` routes to `.remoteComputer(nodeId:)`
            await harness.store.handleInlineKeyboardAction(
                button: FabushiInlineKeyboardButton(text: "Open Mac", callbackData: "computer:node-mac-01"),
                chatId: 601,
                messageId: 11,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertEqual(harness.router.activeRoute, .remoteComputer(nodeId: "node-mac-01"))

            // 3. `web_app_url` -> MiniApp route
            await harness.store.handleInlineKeyboardAction(
                button: FabushiInlineKeyboardButton(text: "Mini App", webAppUrl: "miniapp-notes"),
                chatId: 601,
                messageId: 12,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertEqual(harness.router.activeRoute, .miniApp(id: "miniapp-notes"))

            // 4. `url` -> Instant View reader overlay
            await harness.store.handleInlineKeyboardAction(
                button: FabushiInlineKeyboardButton(text: "Read Docs", url: "https://example.org/guide"),
                chatId: 601,
                messageId: 13,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertEqual(harness.store.overlayController.presentedInstantView?.url, "https://example.org/guide")

            // 5. TRAP #5 honest non-destructive notice for unmapped callback
            harness.router.navigate(to: .chats(chatId: 601, messageId: nil), coordinator: harness.coordinator)
            await harness.store.handleInlineKeyboardAction(
                button: FabushiInlineKeyboardButton(text: "Custom", callbackData: "custom_bot_action:99"),
                chatId: 601,
                messageId: 14,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertEqual(
                harness.router.toastController.activeWindowToast?.message,
                FabushiMessagingStrings.localized(.inlineCallbackPendingNotice, locale: .en)
            )

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 4. `D10-08` Authoritative File Download Failure Toast (NEVER `"Saved"`) & TTL Rollback

    func testD1008FailedFileDownloadShowsFailureToastAndNeverSavedAndTTLRollback() async throws {
        let shouldFailTTL = OSAllocatedUnfairLock<Bool>(initialState: true)

        let harness = try await makeHarness { req in
            if req.path.hasPrefix("/api/v1/media/stream/") {
                return .jsonObject(404, ["error": "not_found"])
            }
            if req.method == "POST" && req.path == "/api/v1/chats/701/ttl" {
                if shouldFailTTL.withLock({ $0 }) {
                    return .jsonObject(500, ["error": "ttl_update_failed"])
                }
                return .jsonObject(200, ["chat_id": 701, "ttl_seconds": 86_400])
            }
            return .jsonObject(200, [:])
        }

        do {
            harness.store.upsertChat(FabushiMessagingChat(chatId: 701, title: "Docs Chat", ttlSeconds: 0))
            harness.router.navigate(to: .chats(chatId: 701, messageId: nil), coordinator: harness.coordinator)

            // File download fails -> MUST show `"Download failed: report.pdf"` and NEVER `"Saved"`
            let downloaded = await harness.store.downloadAttachment(
                fileId: "missing-file-99",
                fileName: "report.pdf",
                chatId: 701,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertNil(downloaded)
            let downloadToast = try XCTUnwrap(harness.router.toastController.activeWindowToast)
            XCTAssertTrue(downloadToast.isError)
            XCTAssertTrue(downloadToast.message.contains("Download failed: report.pdf"))
            XCTAssertFalse(downloadToast.message.localizedCaseInsensitiveContains("saved"))

            // TTL fails -> rolls back to 0 and shows error toast
            let failedTTL = await harness.store.setChatTTL(
                chatId: 701,
                ttlSeconds: 86_400,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertFalse(failedTTL)
            XCTAssertEqual(harness.store.chats.first(where: { $0.chatId == 701 })?.ttlSeconds, 0)
            XCTAssertTrue(harness.router.toastController.activeWindowToast?.isError == true)

            // TTL succeeds -> updates to 86400 and shows success toast
            shouldFailTTL.withLock { $0 = false }
            let okTTL = await harness.store.setChatTTL(
                chatId: 701,
                ttlSeconds: 86_400,
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertTrue(okTTL)
            XCTAssertEqual(harness.store.chats.first(where: { $0.chatId == 701 })?.ttlSeconds, 86_400)
            XCTAssertEqual(harness.router.toastController.activeWindowToast?.isError, false)

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 5. `D10-07` CJK IME Guard, Draft Preservation, Failure Restore & Mid-Upload Generation Fence

    func testD1007IMECompositionGuardDraftPreservationAndMidUploadGenerationFence() async throws {
        // Pure IME composition fence assertions
        XCTAssertFalse(FabushiIMECompositionGuard.shouldAllowReturnToSend(hasMarkedText: true, isShiftPressed: false, text: "nihao"))
        XCTAssertFalse(FabushiIMECompositionGuard.shouldAllowReturnToSend(hasMarkedText: false, isShiftPressed: true, text: "hello"))
        XCTAssertFalse(FabushiIMECompositionGuard.shouldAllowReturnToSend(hasMarkedText: false, isShiftPressed: false, text: "   "))
        XCTAssertTrue(FabushiIMECompositionGuard.shouldAllowReturnToSend(hasMarkedText: false, isShiftPressed: false, text: "你好"))
        XCTAssertFalse(FabushiIMECompositionGuard.shouldAllowCommandOrMentionCommit(hasMarkedText: true))
        XCTAssertTrue(FabushiIMECompositionGuard.shouldAllowCommandOrMentionCommit(hasMarkedText: false))
        XCTAssertNil(FabushiIMECompositionGuard.activeSlashPrefix(text: "/distill", hasMarkedText: true))
        XCTAssertEqual(FabushiIMECompositionGuard.activeSlashPrefix(text: "/distill", hasMarkedText: false), "/distill")
        XCTAssertNil(FabushiIMECompositionGuard.activeSlashPrefix(text: "/Users/alice/main.swift", hasMarkedText: false))

        let sendCalledCount = OSAllocatedUnfairLock<Int>(initialState: 0)
        let shouldFailSend = OSAllocatedUnfairLock<Bool>(initialState: false)

        let harness = try await makeHarness { req in
            if req.method == "PUT" && req.path == "/api/v1/chats/801/draft" {
                return .jsonObject(200, ["chat_id": 801, "text": "Unsent draft", "entities": [], "date_ms": 1_730_000_000_000])
            }
            if req.method == "POST" && req.path == "/api/v1/messages/send" {
                sendCalledCount.withLock { $0 += 1 }
                if shouldFailSend.withLock({ $0 }) {
                    return .jsonObject(500, ["error": "send_failed"])
                }
                return .jsonObject(200, [
                    "message_id": 8801,
                    "pts": 10,
                    "date_ms": 1_730_000_000_000,
                    "sender_id": 1001,
                    "chat_id": 801,
                    "text": "ok",
                    "status": "sent",
                ])
            }
            if req.method == "POST" && req.path == "/api/v1/media/upload" {
                Thread.sleep(forTimeInterval: 0.12)
                return .jsonObject(200, [
                    "file_id": "up-801",
                    "size_bytes": 4,
                    "mime_type": "image/jpeg",
                ])
            }
            return .jsonObject(200, [:])
        }

        do {
            harness.store.upsertChat(FabushiMessagingChat(chatId: 801, title: "Draft Chat"))
            harness.router.navigate(to: .chats(chatId: 801, messageId: nil), coordinator: harness.coordinator)

            // 1. Save unsent draft -> non-composer send MUST preserve draft
            await harness.store.updateComposerDraft(chatId: 801, text: "Unsent draft", persistRemote: true)
            _ = try await harness.store.sendNonComposerMessage(chatId: 801, text: "👍")
            XCTAssertEqual(harness.store.unsentDraftTextByChat[801], "Unsent draft")

            // 2. Permanent send failure restores unsent text + emits chat-scoped error toast
            shouldFailSend.withLock { $0 = true }
            let failedResp = await harness.store.sendComposerMessage(
                chatId: 801,
                text: "Will fail permanently",
                router: harness.router,
                isCompact: false,
                locale: .en
            )
            XCTAssertNil(failedResp)
            XCTAssertEqual(harness.store.unsentDraftTextByChat[801], "Will fail permanently")
            XCTAssertEqual(
                harness.router.toastController.activeWindowToast?.message,
                FabushiMessagingStrings.localized(.messageSendFailedToast, locale: .en)
            )

            // 3. Account switch mid-upload aborts before `messages.sendDurable`!
            shouldFailSend.withLock { $0 = false }
            let sendsBeforeUpload = sendCalledCount.withLock { $0 }
            let uploadTask = Task { @MainActor in
                await harness.store.uploadAndSendAttachment(
                    chatId: 801,
                    fileName: "secret.jpg",
                    mimeType: "image/jpeg",
                    data: Data([0xFF, 0xD8, 0xFF, 0xD9]),
                    contentBuilder: { up in .photo(fileId: up.fileId, caption: nil, width: 10, height: 10) },
                    router: harness.router,
                    isCompact: false,
                    locale: .en
                )
            }
            try await Task.sleep(nanoseconds: 30_000_000)
            _ = try await harness.coordinator.setAuthSession(
                FabushiSessionAuthParams(userId: 2002, username: "bob_m3b", sessionId: "sess-bob", token: "tok-bob")
            )
            let uploadResult = await uploadTask.value
            XCTAssertNil(uploadResult)
            XCTAssertEqual(sendCalledCount.withLock { $0 }, sendsBeforeUpload, "Mid-upload account switch MUST abort sendDurable")

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }

    // MARK: - 6. 900s Grouping, 11+ Cards, Edit, Tombstone, Forward, Reactions, Pins, Polls, Todos, Export & Stats

    func testTranscript900sClustering11PlusCardsStageActionsExportAndStats() async throws {
        let baseMs: Int64 = 1_730_000_000_000
        let msgA = FabushiMessageRecord(messageId: 1, chatId: 901, senderId: 200, text: "First", dateMs: baseMs, isOutgoing: false)
        let msgB = FabushiMessageRecord(messageId: 2, chatId: 901, senderId: 200, text: "Within 900s", dateMs: baseMs + 899_000, isOutgoing: false)
        let msgC = FabushiMessageRecord(messageId: 3, chatId: 901, senderId: 200, text: "After 900s", dateMs: baseMs + 1_900_000, isOutgoing: false)

        let clustered = FabushiMessageClusterCalculator.compute(messages: [msgA, msgB, msgC], isGroupChat: true)
        XCTAssertEqual(clustered.count, 3)
        XCTAssertEqual(clustered[0].position, .first)
        XCTAssertFalse(clustered[0].showsTail)
        XCTAssertTrue(clustered[0].showsSenderHeader)
        XCTAssertEqual(clustered[1].position, .last)
        XCTAssertTrue(clustered[1].showsTail)
        XCTAssertFalse(clustered[1].showsSenderHeader)
        XCTAssertEqual(clustered[2].position, .single)
        XCTAssertTrue(clustered[2].showsTail)

        // Verify ALL 11+ canonical card classifications
        let sampleCards: [(FabushiMessageRecord, FabushiCanonicalCardKind)] = [
            (FabushiMessageRecord(messageId: 10, chatId: 901, senderId: 1, text: "Hello", content: .text(text: "Hello", entities: []), dateMs: baseMs), .text),
            (FabushiMessageRecord(messageId: 11, chatId: 901, senderId: 1, text: "", content: .photo(fileId: "p1", caption: "pic", width: 100, height: 100), dateMs: baseMs), .photo),
            (FabushiMessageRecord(messageId: 12, chatId: 901, senderId: 1, text: "", content: .video(fileId: "v1", duration: 12, width: 100, height: 100, caption: nil), dateMs: baseMs), .video),
            (FabushiMessageRecord(messageId: 13, chatId: 901, senderId: 1, text: "", content: .voice(FabushiVoiceItemPayload(fileId: "vo1", durationSeconds: 5, waveform: [], mimeType: "audio/mp4", sizeBytes: 1024)), dateMs: baseMs), .voice),
            (FabushiMessageRecord(messageId: 14, chatId: 901, senderId: 1, text: "", content: .document(fileId: "d1", fileName: "spec.pdf", mimeType: "application/pdf", sizeBytes: 2048), dateMs: baseMs), .file),
            (FabushiMessageRecord(messageId: 15, chatId: 901, senderId: 1, text: "✨", content: .extended(kind: "sticker", rawJSON: "{\"emoji\":\"✨\"}"), dateMs: baseMs), .sticker),
            (FabushiMessageRecord(messageId: 16, chatId: 901, senderId: 1, text: "", content: .extended(kind: "album", rawJSON: "{\"items\":[]}"), dateMs: baseMs), .album),
            (FabushiMessageRecord(messageId: 17, chatId: 901, senderId: 1, text: "", content: .extended(kind: "location", rawJSON: "{\"latitude\":37.33,\"longitude\":-122.03}"), dateMs: baseMs), .location),
            (FabushiMessageRecord(messageId: 18, chatId: 901, senderId: 1, text: "", content: .poll(FabushiPollCard(pollId: 1, question: "Q?", options: [])), dateMs: baseMs), .poll),
            (FabushiMessageRecord(messageId: 19, chatId: 901, senderId: 1, text: "", content: .todo(FabushiTodoCard(id: "t1", title: "Tasks", items: [])), dateMs: baseMs), .todo),
            (FabushiMessageRecord(messageId: 20, chatId: 901, senderId: 1, text: "Joined", content: .extended(kind: "service", rawJSON: "{}"), dateMs: baseMs), .service),
            (FabushiMessageRecord(messageId: 21, chatId: 901, senderId: 1, text: "Call", content: .extended(kind: "call", rawJSON: "{\"status\":\"missed\"}"), dateMs: baseMs), .call),
            (FabushiMessageRecord(messageId: 22, chatId: 901, senderId: 1, text: "Story", content: .extended(kind: "story_mention", rawJSON: "{}"), dateMs: baseMs), .storyMention),
            (FabushiMessageRecord(messageId: 23, chatId: 901, senderId: 1, text: "", content: .invoice(FabushiInvoiceCard(invoiceId: "inv1", title: "Pro", descriptionText: "Plan", currency: "USD", amountCents: 999)), dateMs: baseMs), .invoice),
            (FabushiMessageRecord(messageId: 24, chatId: 901, senderId: 1, text: "Takeover", content: .extended(kind: "browser_takeover_requested", rawJSON: "{\"session_id\":\"s1\"}"), dateMs: baseMs), .browserTakeover),
            (FabushiMessageRecord(messageId: 25, chatId: 901, senderId: 1, text: "", content: .encrypted(rawEnvelopeJSON: "{}"), dateMs: baseMs), .encrypted),
        ]
        for (record, expectedKind) in sampleCards {
            XCTAssertEqual(FabushiCanonicalCardKind.classify(record), expectedKind)
        }

        let forwardedBodies = OSAllocatedUnfairLock<[String]>(initialState: [])

        let harness = try await makeHarness { req in
            if req.method == "PUT" && req.path == "/api/v1/chats/901/messages/100" {
                return .jsonObject(200, ["edited": true])
            }
            if req.method == "DELETE" && req.path == "/api/v1/chats/901/messages/100" {
                return .jsonObject(200, ["deleted": true])
            }
            if req.method == "POST" && req.path == "/api/v1/chats/901/messages/100/reactions" {
                return .jsonObject(200, [
                    "success": true,
                    "chat_id": 901,
                    "message_id": 100,
                    "pts": 55,
                    "reactions": [["emoji": "🔥", "count": 2, "is_chosen_by_me": true]],
                    "details": [],
                ])
            }
            if req.method == "POST" && req.path == "/api/v1/chats/901/pins" {
                return .jsonObject(200, [
                    "chat_id": 901,
                    "message_id": 100,
                    "pinned_by": 1001,
                    "pinned_at_ms": 1_730_000_000_000,
                    "preview_text": "Edited body",
                ])
            }
            if req.method == "POST" && req.path == "/api/v1/messages/send" {
                forwardedBodies.withLock { $0.append(req.bodyString) }
                return .jsonObject(200, [
                    "message_id": 909,
                    "pts": 60,
                    "date_ms": 1_730_000_000_000,
                    "sender_id": 1001,
                    "chat_id": 902,
                    "text": "Edited body",
                    "status": "sent",
                ])
            }
            if req.method == "POST" && req.path == "/api/v1/polls/555/vote" {
                return .jsonObject(200, [
                    "success": true,
                    "pts": 61,
                    "poll": [
                        "poll_id": 555,
                        "question": "Favorite language?",
                        "options": [
                            ["option_id": 0, "text": "Swift", "vote_count": 1, "is_chosen": true, "percentage": 100.0],
                            ["option_id": 1, "text": "Rust", "vote_count": 0, "is_chosen": false, "percentage": 0.0],
                        ],
                        "total_voters": 1,
                        "is_closed": false,
                    ],
                ])
            }
            if req.method == "GET" && req.path == "/api/v1/chats/901/export" {
                return .jsonObject(200, ["chat_id": 901, "messages": [["id": 100]]])
            }
            if req.method == "GET" && req.path == "/api/v1/chats/901/statistics" {
                return .jsonObject(200, [
                    "chat_id": 901,
                    "title": "Design Group",
                    "member_count": 12,
                    "total_messages": 420,
                    "messages_last_7d": 88,
                    "total_reactions": 64,
                    "active_posters_7d": 7,
                    "growth_rate_pct": 14.5,
                    "daily_series": [
                        ["day": "2026-11-24", "timestamp_ms": 1_730_000_000_000, "messages": 18, "reactions": 9, "active_senders": 5],
                    ],
                ])
            }
            return .jsonObject(200, [:])
        }

        do {
            harness.store.upsertChat(FabushiMessagingChat(chatId: 901, title: "Design Bot Group", kind: "group", isGroup: true, isBot: false))
            harness.store.upsertChat(FabushiMessagingChat(chatId: 902, title: "Target Chat"))

            // Seed message 100 + poll message 101 + todo message 102 via real domain events
            _ = harness.store.ingestDomainEvent(FabushiDomainEvent(
                type: "newMessage",
                generation: harness.coordinator.accountScopeGeneration,
                timestampMs: baseMs,
                rawPayloadJSON: """
                {"id":100,"chat_id":901,"sender_id":1001,"date":1730000000000,"is_outgoing":true,"content":{"type":"Text","data":{"text":"Initial body","entities":[]}}}
                """
            ))
            _ = harness.store.ingestDomainEvent(FabushiDomainEvent(
                type: "newMessage",
                generation: harness.coordinator.accountScopeGeneration,
                timestampMs: baseMs + 1000,
                rawPayloadJSON: """
                {"id":101,"chat_id":901,"sender_id":1001,"date":1730000001000,"content":{"type":"Poll","data":{"poll_id":555,"question":"Favorite language?","options":[{"option_id":0,"text":"Swift","vote_count":0,"is_chosen":false,"percentage":0},{"option_id":1,"text":"Rust","vote_count":0,"is_chosen":false,"percentage":0}],"total_voters":0,"is_closed":false}}}
                """
            ))
            _ = harness.store.ingestDomainEvent(FabushiDomainEvent(
                type: "newMessage",
                generation: harness.coordinator.accountScopeGeneration,
                timestampMs: baseMs + 2000,
                rawPayloadJSON: """
                {"id":102,"chat_id":901,"sender_id":1001,"date":1730000002000,"content":{"type":"Todo","data":{"id":"todo-1","title":"Checklist","items":[{"id":"i1","title":"Ship M3b","completed":false}],"completed_count":0}}}
                """
            ))

            // Edit in-place
            let edited = await harness.store.editMessage(chatId: 901, messageId: 100, newText: "Edited body")
            XCTAssertTrue(edited)
            XCTAssertEqual(harness.store.messagesByChat[901]?.first(where: { $0.messageId == 100 })?.text, "Edited body")
            XCTAssertNotNil(harness.store.messagesByChat[901]?.first(where: { $0.messageId == 100 })?.editDateMs)

            // PTS-fenced reaction
            let reactions = await harness.store.toggleReaction(chatId: 901, messageId: 100, emoji: "🔥")
            XCTAssertEqual(reactions.first?.emoji, "🔥")
            XCTAssertEqual(reactions.first?.isChosenByMe, true)

            // Pin message
            let pinned = await harness.store.pinMessage(chatId: 901, messageId: 100, router: harness.router, locale: .en)
            XCTAssertTrue(pinned)
            XCTAssertEqual(harness.store.pinnedMessagesByChat[901]?.first?.messageId, 100)

            // Multi-forward via `sendDurable` preserving target draft
            await harness.store.updateComposerDraft(chatId: 902, text: "Keep target draft", persistRemote: false)
            let forwarded = await harness.store.forwardMessages(fromChatId: 901, messageIds: [100], toChatId: 902)
            XCTAssertEqual(forwarded, 1)
            XCTAssertEqual(harness.store.unsentDraftTextByChat[902], "Keep target draft")
            let fwdBody = try XCTUnwrap(forwardedBodies.withLock { $0.first })
            XCTAssertTrue(fwdBody.contains("\"forward_from_chat_id\":901"))
            XCTAssertTrue(fwdBody.contains("\"forward_from_msg_id\":100"))

            // Poll vote & Todo toggle
            let voted = await harness.store.votePoll(chatId: 901, messageId: 101, pollId: 555, optionIds: [0])
            XCTAssertEqual(voted?.totalVoters, 1)
            XCTAssertEqual(voted?.options.first?.isChosen, true)

            harness.store.toggleTodoItem(chatId: 901, messageId: 102, itemId: "i1")
            if case .todo(let updatedTodo)? = harness.store.messagesByChat[901]?.first(where: { $0.messageId == 102 })?.content {
                XCTAssertEqual(updatedTodo.completedCount, 1)
                XCTAssertEqual(updatedTodo.items.first?.completed, true)
            } else {
                XCTFail("Expected updated todo card")
            }

            // Delete for everyone renders tombstone
            let deleted = await harness.store.deleteMessages(chatId: 901, messageIds: [100], forEveryone: true)
            XCTAssertTrue(deleted)
            XCTAssertEqual(harness.store.messagesByChat[901]?.first(where: { $0.messageId == 100 })?.isDeletedTombstone, true)

            // Export JSON & Statistics
            let exported = try await harness.store.exportChatHistoryJSON(chatId: 901)
            XCTAssertFalse(exported.isEmpty)
            let stats = try await harness.store.client.getChannelStatistics(chatId: 901)
            XCTAssertEqual(stats.totalMessages, 420)
            XCTAssertEqual(stats.dailySeries.count, 1)

            // `D14-04`: Group bot activity displays cleanly without flipping `isBot` or polluting `knownBotIds`
            harness.store.applyBotActivityEvent(chatId: 901, botUserId: 777, state: "thinking", detail: "Inspecting diff")
            XCTAssertTrue(harness.store.canDisplayBotActivity(for: 901))
            XCTAssertEqual(harness.store.chats.first(where: { $0.chatId == 901 })?.isBot, false)
            XCTAssertFalse(harness.store.knownBotIds.contains(901))

            await harness.tearDown()
        } catch {
            await harness.tearDown()
            throw error
        }
    }
}
