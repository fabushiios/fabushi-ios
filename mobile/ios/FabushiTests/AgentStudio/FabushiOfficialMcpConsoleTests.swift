import Foundation
import os
import XCTest
@testable import Fabushi

@MainActor
final class FabushiOfficialMcpConsoleTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m4b_mcp_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain(_ suffix: String) -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4b.mcp.\(suffix).\(UUID().uuidString)",
            account: "m4b-mcp-\(suffix)"
        )
    }

    private func makeRouter() -> ShellNavigationRouter {
        ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
    }

    // MARK: - 1. 14 Scopes, 31 Official MCP Tools, QR Pair Parser & Exact `official_mcp.rs` Audit Redactor

    func testOfficialCatalog14Scopes31ToolsQrParserAndExactAuditRedactor() {
        XCTAssertEqual(FabushiMcpOfficialCatalog.supportedScopes.count, 14)
        XCTAssertEqual(Set(FabushiMcpOfficialCatalog.supportedScopes).count, 14)

        XCTAssertEqual(FabushiMcpOfficialCatalog.officialTools.count, 31)
        XCTAssertEqual(Set(FabushiMcpOfficialCatalog.officialTools.map(\.name)).count, 31)

        let supportedSet = Set(FabushiMcpOfficialCatalog.supportedScopes)
        let coveredScopes = Set(FabushiMcpOfficialCatalog.officialTools.map(\.scope))
        XCTAssertEqual(coveredScopes, supportedSet, "Every one of the 14 canonical OAuth scopes must be covered by the 31 official MCP tools")
        XCTAssertEqual(FabushiMcpOfficialCatalog.requiredScope(for: "terminal_exec"), "terminal:exec")
        XCTAssertEqual(FabushiMcpOfficialCatalog.requiredScope(for: "send_message"), "messaging:write")
        XCTAssertEqual(FabushiMcpOfficialCatalog.requiredScope(for: "miniapp_list"), "miniapp:control")

        // QR pair token extraction (raw, custom URL scheme, HTTPS qr-poll URL)
        XCTAssertEqual(FabushiMcpQrPairParser.extractPairToken(from: "pair_abc12345"), "pair_abc12345")
        XCTAssertEqual(
            FabushiMcpQrPairParser.extractPairToken(from: "fabushi://pair?pair_token=pair_scheme_77"),
            "pair_scheme_77"
        )
        XCTAssertEqual(
            FabushiMcpQrPairParser.extractPairToken(from: "https://srv.example.com/oauth/authorize/qr-poll?tx_id=otx_1&pair_token=pair_https_99"),
            "pair_https_99"
        )
        XCTAssertNil(FabushiMcpQrPairParser.extractPairToken(from: "   "))

        // Exact 5-key `<N chars>` redaction matching `official_mcp.rs:L771-L788` (incl. Unicode CJK scalar count)
        let rawArgs = """
        {
          "code": "print('hi')",
          "text": "你好世界",
          "content": "secret file body",
          "data": "base64payload==",
          "password": "my-pass-word",
          "node_id": "local:1:mac",
          "note": "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
        }
        """
        let redacted = FabushiMcpAuditRedactor.redactArgsSummary(rawArgs)
        XCTAssertTrue(redacted.contains("\"code\":\"<11 chars>\""))
        XCTAssertTrue(redacted.contains("\"text\":\"<4 chars>\""))
        XCTAssertTrue(redacted.contains("\"content\":\"<16 chars>\""))
        XCTAssertTrue(redacted.contains("\"data\":\"<15 chars>\""))
        XCTAssertTrue(redacted.contains("\"password\":\"<12 chars>\""))
        XCTAssertTrue(redacted.contains("\"node_id\":\"local:1:mac\""))
        XCTAssertTrue(redacted.contains("[REDACTED:JWT]"))
        XCTAssertFalse(redacted.contains("my-pass-word"))
        XCTAssertFalse(redacted.contains("你好世界"))
        XCTAssertFalse(redacted.contains("eyJhbGciOiJIUzI1NiJ9"))

        // Exact 280-char truncation (`277 + "..."`)
        let longPayload = "{\"path\":\"/\(String(repeating: "a", count: 350))\"}"
        let truncated = FabushiMcpAuditRedactor.redactArgsSummary(longPayload)
        XCTAssertEqual(truncated.unicodeScalars.count, 280)
        XCTAssertTrue(truncated.hasSuffix("..."))
    }

    // MARK: - 2. Grants, Revoke, QR-Poll Accept, Kill-Switch, `stdio` Fail-Closed, Local Registry & Audit via Loopback

    func testGrantsRevokeQrPairAcceptKillSwitchStdioBlockedAndAuditViaLoopback() async throws {
        let grantsState = OSAllocatedUnfairLock<[String]>(initialState: ["fam_1", "fam_2"])
        let serversState = OSAllocatedUnfairLock<[String]>(initialState: [])

        let server = try M2bLoopbackHTTPTestServer { req in
            if req.method == "GET", req.path == "/api/v1/mcp/grants" {
                let ids = grantsState.withLock { $0 }
                let items = ids.map { fid in
                    """
                    {
                      "family_id": "\(fid)",
                      "client_id": "client_\(fid)",
                      "client_name": "IDE Client \(fid)",
                      "scopes": ["devices:read", "terminal:exec"],
                      "allowed_node_ids": ["*"],
                      "created_at_ms": 1750000000000,
                      "last_used_ms": 1750000100000,
                      "expires_at_ms": 1752592000000
                    }
                    """
                }.joined(separator: ",")
                return .json(200, "{\"grants\":[\(items)]}")
            }
            if req.method == "DELETE", req.path.hasPrefix("/api/v1/mcp/grants/") {
                let fid = String(req.path.dropFirst("/api/v1/mcp/grants/".count))
                grantsState.withLock { $0.removeAll { $0 == fid } }
                return .json(200, #"{"revoked":true}"#)
            }
            if req.method == "POST", req.path == "/api/v1/auth/pair/accept" {
                return .json(200, #"{}"#)
            }
            if req.method == "POST", req.path == "/api/v1/remote/sessions/disconnect-all" {
                return .json(200, #"{"disconnected":2,"reason":"user_kill_switch"}"#)
            }
            if req.method == "POST", req.path == "/api/v1/mcp/servers" {
                serversState.withLock { $0.append(req.bodyString) }
                return .json(
                    200,
                    #"{"id":"srv_cloud_1","name":"CloudDocs","transport":"streamable_http","url":"https://mcp.example.com/mcp","status":"connected"}"#
                )
            }
            if req.method == "GET", req.path == "/api/v1/mcp/servers" {
                let count = serversState.withLock { $0.count }
                if count > 0 {
                    return .json(
                        200,
                        #"[{"id":"srv_cloud_1","name":"CloudDocs","transport":"streamable_http","url":"https://mcp.example.com/mcp","status":"connected"}]"#
                    )
                }
                return .json(200, "[]")
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
                    userId: 505,
                    username: "mcp_owner",
                    sessionId: "sess-505",
                    token: "tok-505",
                    firstName: "Owner"
                )
            )

            let hub = FabushiAgentStudioLogoutCleanupHub()
            let vm = FabushiMcpViewModel(coordinator: coordinator, router: makeRouter(), cleanupHub: hub)

            // 1. OAuth 2.1 Grants list & revoke
            await vm.loadGrants()
            XCTAssertEqual(vm.grants.count, 2)
            XCTAssertTrue(vm.grants.first?.allowsAllDevices == true)

            let revoked = await vm.revokeGrant(familyId: "fam_1")
            XCTAssertTrue(revoked)
            XCTAssertEqual(vm.grants.map(\.familyId), ["fam_2"])
            XCTAssertEqual(server.requests(method: "DELETE", pathPrefix: "/api/v1/mcp/grants/fam_1").count, 1)

            // 2. QR-Poll Login Approval (`auth.qrPairAccept`)
            let accepted = await vm.approveQrPairLogin(
                rawPayload: "https://srv.example.com/oauth/authorize/qr-poll?tx_id=otx_9&pair_token=pair_mcp_88"
            )
            XCTAssertTrue(accepted)
            XCTAssertEqual(vm.lastApprovedPairToken, "pair_mcp_88")
            let pairReqs = server.requests(method: "POST", pathPrefix: "/api/v1/auth/pair/accept")
            XCTAssertEqual(pairReqs.count, 1)
            XCTAssertTrue(pairReqs.first?.bodyString.contains("pair_mcp_88") == true)

            // 3. Global Emergency Kill-Switch (`remote.sessionsDisconnectAll`)
            let killOk = await vm.triggerEmergencyKillSwitch()
            XCTAssertTrue(killOk)
            XCTAssertEqual(
                server.requests(method: "POST", pathPrefix: "/api/v1/remote/sessions/disconnect-all").count,
                1
            )

            // 4. `mcp.registerServer` blocks `stdio` fail-closed BEFORE hitting network (`IOS-MAIN-INV-04`),
            //    while HTTPS `streamable_http` succeeds!
            do {
                _ = try await vm.registerServer(
                    name: "LocalBadStdio",
                    transport: "stdio",
                    command: "npx -y @modelcontextprotocol/server-filesystem"
                )
                XCTFail("Expected FabushiHostError.capabilityUnavailable for stdio transport on iOS")
            } catch let hostError as FabushiHostError {
                if case .capabilityUnavailable = hostError {
                    XCTAssertNotNil(vm.lastSandboxNotice)
                } else {
                    XCTFail("Expected .capabilityUnavailable, got \(hostError)")
                }
            }
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/mcp/servers").count, 0)

            _ = try await vm.registerServer(
                name: "CloudDocs",
                transport: "streamable_http",
                url: "https://mcp.example.com/mcp"
            )
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/mcp/servers").count, 1)
            XCTAssertEqual(vm.remoteServers.first?.name, "CloudDocs")

            // 5. Local Registry & Permission Dock (`mcp.localRegistryList`, `mcp.permissionDockResolve`)
            await vm.loadLocalRegistryTools()
            let dockResp = await vm.resolveLocalPermissionDock(requestId: "req_local_1", approved: true)
            XCTAssertEqual(dockResp?.requestId, "req_local_1")
            XCTAssertEqual(dockResp?.approved, true)

            // 6. Real Audit Log probe (`mcp.listAudit` returns unknown_command -> honest M6 flag, ZERO fake rows)
            //    + reflects real local SQLite approval record redacted via FabushiMcpAuditRedactor
            struct EnqueueParams: Encodable, Sendable {
                let session_id: String
                let command: String
                let risk_level: String
                let pattern_name: String
            }
            let _: FabushiStudioApprovalRecordDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.fabushi.method("approvalEnqueue"),
                params: EnqueueParams(
                    session_id: "sess_audit_1",
                    command: "terminal_exec Bearer secret_token_value_12345",
                    risk_level: "high",
                    pattern_name: "terminal_exec"
                )
            )
            await vm.loadAuditLog()
            XCTAssertTrue(vm.isServerAuditPendingM6)
            XCTAssertEqual(vm.auditRecords.count, 1)
            XCTAssertEqual(vm.auditRecords.first?.toolName, "terminal_exec")
            XCTAssertFalse(vm.auditRecords.first?.argsSummary.contains("secret_token_value_12345") ?? true)
            XCTAssertTrue(vm.auditRecords.first?.argsSummary.contains("[REDACTED:BEARER_TOKEN]") == true)

            // 7. Logout cleanup wipes all MCP console state
            vm.pairTokenInput = "pair_unsent_token"
            hub.resetForDepartingAccount("505")
            XCTAssertTrue(vm.grants.isEmpty)
            XCTAssertTrue(vm.remoteServers.isEmpty)
            XCTAssertTrue(vm.auditRecords.isEmpty)
            XCTAssertEqual(vm.pairTokenInput, "")
            XCTAssertNil(vm.lastApprovedPairToken)
            XCTAssertNil(vm.lastKillSwitchResult)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
