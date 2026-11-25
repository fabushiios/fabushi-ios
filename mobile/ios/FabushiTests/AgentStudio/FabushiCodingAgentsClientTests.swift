import Foundation
import os
import XCTest
@testable import Fabushi

@MainActor
final class FabushiCodingAgentsClientTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m4b_coding_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain(_ suffix: String) -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4b.coding.\(suffix).\(UUID().uuidString)",
            account: "m4b-coding-\(suffix)"
        )
    }

    private func makeRouter() -> ShellNavigationRouter {
        ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
    }

    // MARK: - 1. All 6 Providers Catalog & Alias Parsing

    func testSixProvidersCatalogAndAliasParsing() async throws {
        let dir = try makeTempDir("catalog")
        let kc = makeIsolatedKeychain("catalog")
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let vm = FabushiCodingViewModel(chatId: 901, coordinator: coordinator, router: makeRouter())
            await vm.loadProvidersCatalog()

            XCTAssertEqual(vm.providersCatalog.count, 6)
            XCTAssertEqual(
                vm.providersCatalog.map(\.kind),
                [.claudeCode, .codex, .openCode, .cursorCli, .antigravity, .acpCustom]
            )
            XCTAssertEqual(
                vm.providersCatalog.filter(\.supportsAcp).map(\.kind),
                [.openCode, .cursorCli, .acpCustom]
            )

            let parsedClaude = await vm.parseProvider("claude")
            XCTAssertEqual(parsedClaude?.kind, .claudeCode)
            XCTAssertEqual(parsedClaude?.supportsAcp, false)

            let parsedCursor = await vm.parseProvider("cursor-agent")
            XCTAssertEqual(parsedCursor?.kind, .cursorCli)
            XCTAssertEqual(parsedCursor?.supportsAcp, true)

            let parsedAgy = await vm.parseProvider("agy")
            XCTAssertEqual(parsedAgy?.kind, .antigravity)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 2. BotFather NL, Coding Bots CRUD, Remote Hosts & Remote Slash Commands via Loopback

    func testBotFatherNlCodingBotsCrudAndRemoteHostProbeViaLoopback() async throws {
        let botPermissionMode = OSAllocatedUnfairLock<String>(initialState: "ask")

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "POST", req.path == "/api/v1/botfather/nl-create" {
                return .json(
                    200,
                    """
                    {
                      "bot": {
                        "bot_user_id": 600101,
                        "owner_user_id": 101,
                        "username": "claude_code_bot",
                        "display_name": "Claude Repo Bot",
                        "description": "Remote coding specialist",
                        "coding_agent": {
                          "provider_kind": "claude_code",
                          "workspace_path": "/srv/repo/fabushi",
                          "use_worktree": true,
                          "permission_mode": "ask"
                        }
                      },
                      "bot_token": "bot-tok-1",
                      "companion_chat_id": 9901,
                      "summary_markdown": "Created @claude_code_bot bound to /srv/repo/fabushi"
                    }
                    """
                )
            }
            if req.method == "GET", req.path == "/api/v1/fabushi-bots" {
                let mode = botPermissionMode.withLock { $0 }
                return .json(
                    200,
                    """
                    [
                      {
                        "bot_user_id": 600101,
                        "owner_user_id": 101,
                        "agent_id": "coding_claude",
                        "username": "claude_code_bot",
                        "display_name": "Claude Repo Bot",
                        "preferred_model": "default",
                        "coding_agent": {
                          "provider_kind": "claude_code",
                          "workspace_path": "/srv/repo/fabushi",
                          "use_worktree": true,
                          "permission_mode": "\(mode)"
                        }
                      }
                    ]
                    """
                )
            }
            if req.method == "POST", req.path == "/api/v1/fabushi-bots" {
                return .json(
                    200,
                    """
                    {
                      "bot": {
                        "bot_user_id": 600102,
                        "owner_user_id": 101,
                        "agent_id": "coding_codex",
                        "username": "codex_bot",
                        "display_name": "Codex Builder",
                        "coding_agent": {
                          "provider_kind": "codex",
                          "workspace_path": "/srv/repo/app",
                          "use_worktree": true,
                          "permission_mode": "auto_edit"
                        }
                      },
                      "registry": {
                        "bot_user_id": 600102,
                        "owner_user_id": 101,
                        "username": "codex_bot",
                        "display_name": "Codex Builder"
                      },
                      "bot_token": "bot-tok-2",
                      "companion_chat_id": 9902
                    }
                    """
                )
            }
            if req.method == "PATCH", req.path == "/api/v1/fabushi-bots/600101" {
                botPermissionMode.withLock { $0 = "auto_edit" }
                return .json(
                    200,
                    """
                    {
                      "bot_user_id": 600101,
                      "owner_user_id": 101,
                      "agent_id": "coding_claude",
                      "username": "claude_code_bot",
                      "display_name": "Claude Repo Bot",
                      "preferred_model": "deepseek-chat",
                      "coding_agent": {
                        "provider_kind": "claude_code",
                        "workspace_path": "/srv/repo/fabushi",
                        "use_worktree": true,
                        "permission_mode": "auto_edit"
                      }
                    }
                    """
                )
            }
            if req.method == "GET", req.path == "/api/v1/computers" {
                return .json(
                    200,
                    """
                    {
                      "computers": [
                        {
                          "node_id": "local:101:mac-studio",
                          "owner_user_id": 101,
                          "kind": "local_desktop",
                          "display_name": "Mac Studio M4",
                          "ip_address": "100.64.0.10",
                          "online": true
                        }
                      ],
                      "pending_approvals": []
                    }
                    """
                )
            }
            if req.method == "POST", req.path == "/api/v1/hermes/command" {
                return .json(
                    200,
                    """
                    {
                      "handled": true,
                      "command": "/diff",
                      "session_id": "thr_1",
                      "output": "diff --git a/App.swift b/App.swift\\n+let ready = true",
                      "reply_text": "diff --git a/App.swift b/App.swift\\n+let ready = true",
                      "inline_keyboard": [],
                      "effects": []
                    }
                    """
                )
            }
            return .json(404, #"{"error":"not_found"}"#)
        }
        defer { server.stop() }

        let dir = try makeTempDir("loopback")
        let kc = makeIsolatedKeychain("loopback")
        defer {
            try? kc.deletePassphrase()
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
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 101,
                    username: "alice",
                    sessionId: "sess-101",
                    token: "tok-101",
                    firstName: "Alice"
                )
            )

            let vm = FabushiCodingViewModel(chatId: 9901, coordinator: coordinator, router: makeRouter())

            // BotFather NL create
            let nlResp = await vm.createBotViaBotFatherNL(prompt: "Create a Claude Code bot for /srv/repo/fabushi")
            XCTAssertEqual(nlResp?.bot.username, "claude_code_bot")
            XCTAssertEqual(vm.codingBots.count, 1)
            // D10-09 normalized model check ("default" -> "")
            XCTAssertEqual(vm.codingBots.first?.normalizedPreferredModel, "")

            // Explicit createBot & patchBot
            let created = await vm.createCodingBot(
                FabushiCodingCreateBotRequest(
                    displayName: "Codex Builder",
                    codingAgent: FabushiCodingAgentBotConfigDTO(
                        providerKind: .codex,
                        workspacePath: "/srv/repo/app",
                        useWorktree: true,
                        permissionMode: .autoEdit
                    )
                )
            )
            XCTAssertEqual(created?.bot.botUserId, 600102)

            let patched = await vm.patchCodingBot(
                botId: 600101,
                preferredModel: "deepseek-chat",
                codingAgent: FabushiCodingAgentBotConfigDTO(
                    providerKind: .claudeCode,
                    workspacePath: "/srv/repo/fabushi",
                    useWorktree: true,
                    permissionMode: .autoEdit
                )
            )
            XCTAssertEqual(patched?.codingAgent?.permissionMode, .autoEdit)

            // Remote coding hosts (`computers.list`)
            let hosts = await vm.probeRemoteCodingHosts()
            XCTAssertEqual(hosts.count, 1)
            XCTAssertEqual(hosts.first?.displayName, "Mac Studio M4")

            // Remote `/diff` command via `fabushi.runCommand`
            let diffResp = await vm.dispatchRemoteCodingCommand("/diff")
            XCTAssertEqual(diffResp?.handled, true)
            XCTAssertTrue(vm.streamingDiffPreview.contains("diff --git"))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 3. `D10-02` Strict `owner_user_id` Isolation Across Accounts 101 & 202 + Logout Wipe

    func testD1002StrictOwnerUserIdIsolationAcrossAccountsAndLogoutWipe() async throws {
        let dir = try makeTempDir("d1002_isolation")
        let kc = makeIsolatedKeychain("d1002_isolation")
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let hub = FabushiAgentStudioLogoutCleanupHub()
            let vm = FabushiCodingViewModel(chatId: 5001, coordinator: coordinator, router: makeRouter(), cleanupHub: hub)

            // Account 101 writes thread & checkpoint
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 101,
                    username: "alice_101",
                    sessionId: "sess-101",
                    token: "tok-101",
                    firstName: "Alice"
                )
            )
            let upsert101 = await vm.upsertThread(
                FabushiCodingThreadUpsertRequest(
                    botUserId: 777,
                    chatId: 5001,
                    threadId: "thr_shared_id",
                    workspacePath: "/repos/alice_secret",
                    worktreePath: "/repos/alice_secret/.worktrees/feat",
                    branchName: "feat/alice",
                    useWorktree: true,
                    activeModel: "claude-sonnet-4-5",
                    permissionMode: .autoEdit,
                    turnCount: 3,
                    lastSummary: "Alice private patch",
                    botProviderKind: .claudeCode
                )
            )
            XCTAssertEqual(upsert101?.ownerUserId, 101)
            XCTAssertEqual(vm.activeThread?.ownerUserId, 101)
            XCTAssertEqual(vm.activeThread?.branchName, "feat/alice")

            let cp101 = await vm.saveCheckpoint(
                threadId: "thr_shared_id",
                turnIdx: 1,
                beforeRef: "aaa111",
                afterRef: "bbb222",
                diffStat: FabushiCodingDiffStatDTO(
                    filesChanged: 2,
                    insertions: 40,
                    deletions: 5,
                    changedFiles: ["Sources/Alice.swift", "Tests/AliceTests.swift"]
                ),
                promptPreview: "Implement Alice feature",
                replyPreview: "Done"
            )
            XCTAssertEqual(cp101?.ownerUserId, 101)
            XCTAssertEqual(vm.checkpoints.count, 1)

            // Switch to Account 202: MUST NOT see Account 101's thread or checkpoints!
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 202,
                    username: "bob_202",
                    sessionId: "sess-202",
                    token: "tok-202",
                    firstName: "Bob"
                )
            )
            vm.syncAccountScopeIfNeeded()
            XCTAssertNil(vm.activeThread)
            XCTAssertTrue(vm.checkpoints.isEmpty)

            let bobThread = await vm.loadThread(botUserId: 777, chatId: 5001)
            XCTAssertNil(bobThread, "Account 202 must NEVER read Account 101's coding thread")

            let bobCheckpoints = await vm.loadCheckpoints(threadId: "thr_shared_id")
            XCTAssertTrue(bobCheckpoints.isEmpty, "Account 202 must NEVER read Account 101's checkpoints")

            // Account 202 writes its own thread on the exact same (botUserId, chatId)
            let upsert202 = await vm.upsertThread(
                FabushiCodingThreadUpsertRequest(
                    botUserId: 777,
                    chatId: 5001,
                    threadId: "thr_shared_id",
                    workspacePath: "/repos/bob_app",
                    branchName: "feat/bob",
                    useWorktree: false,
                    permissionMode: .ask,
                    turnCount: 1,
                    botProviderKind: .codex
                )
            )
            XCTAssertEqual(upsert202?.ownerUserId, 202)
            XCTAssertEqual(vm.activeThread?.branchName, "feat/bob")

            // Switch back to Account 101 -> Alice's original thread & checkpoint remain intact
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 101,
                    username: "alice_101",
                    sessionId: "sess-101-b",
                    token: "tok-101-b",
                    firstName: "Alice"
                )
            )
            let restoredAlice = await vm.loadThread(botUserId: 777, chatId: 5001)
            XCTAssertEqual(restoredAlice?.ownerUserId, 101)
            XCTAssertEqual(restoredAlice?.branchName, "feat/alice")
            XCTAssertEqual(vm.checkpoints.count, 1)
            XCTAssertEqual(vm.checkpoints.first?.afterRef, "bbb222")

            // Logout cleanup wipes all in-memory coding state
            hub.resetForDepartingAccount("101")
            XCTAssertNil(vm.activeThread)
            XCTAssertTrue(vm.checkpoints.isEmpty)
            XCTAssertTrue(vm.codingBots.isEmpty)
            XCTAssertTrue(vm.remoteComputers.isEmpty)
            XCTAssertTrue(vm.pendingApprovals.isEmpty)
            XCTAssertEqual(vm.streamingDiffPreview, "")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 4. Strict `IOS-MAIN-INV-04` Fail-Closed Local Subprocess Actions

    func testLocalSubprocessActionsFailClosedWithCapabilityUnavailable() async throws {
        let dir = try makeTempDir("fail_closed")
        let kc = makeIsolatedKeychain("fail_closed")
        defer {
            try? kc.deletePassphrase()
            try? FileManager.default.removeItem(at: dir)
        }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: kc)
        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )

        do {
            let vm = FabushiCodingViewModel(chatId: 0, coordinator: coordinator, router: makeRouter())
            for forbiddenAction in ["probeLocal", "startLocalTurn", "localControl"] {
                let blocked = await vm.probeLocalSubprocessFailClosed(action: forbiddenAction)
                XCTAssertTrue(blocked, "Expected coding.\(forbiddenAction) to fail closed with capabilityUnavailable on iOS")
            }
            XCTAssertNotNil(vm.lastSandboxNotice)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
