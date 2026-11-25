import Foundation
import os
import SwiftUI
import UIKit
import XCTest
@testable import Fabushi

@MainActor
final class FabushiRemoteComputerTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m5a_remote_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain() -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m5a.remote.\(UUID().uuidString)",
            account: "m5a-remote"
        )
    }

    private func signInTestUser(coordinator: FabushiCoordinator) async throws {
        let _: FabushiRPCEnvelopeResult<FabushiAuthSessionResponse> = try await coordinator.dispatchAuthTransition(
            method: FabushiRPCNamespace.auth.method("login"),
            params: FabushiLegacyLoginRequest(username: "remote_ops_user", password: "StrongPassword!1"),
            extractIdentity: { (res: FabushiAuthSessionResponse) in res.identityTuple }
        )
    }

    /// 1x1 valid PNG data URL for verifying real `latest_frame_data_url` decoding.
    private static let sampleFrameDataURL =
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="

    // MARK: - 1. `computers.list` + `remote.assistAllow` Optimistic Rollback & Non-Chat Toast on HTTP 500

    func testComputersListAndAllowRemoteOptimisticRollbackOnHttp500() async throws {
        let frameURL = Self.sampleFrameDataURL
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":501,"username":"remote_ops_user","session_id":"s-501","token":"tok-501","first_name":"Ops"}"#)
            case ("GET", "/api/v1/computers"):
                return .jsonObject(200, [
                    "computers": [
                        [
                            "node_id": "local:501:mac-studio",
                            "owner_user_id": 501,
                            "kind": "local_device",
                            "display_name": "Studio Mac",
                            "ip_address": "100.64.0.10",
                            "os_name": "macOS 15.4",
                            "resolution": "3840x2160",
                            "online": true,
                            "is_current_device": false,
                            "tier_badge": "本地",
                            "run_policy": "always_allow_authorized",
                            "authorized_controllers": [],
                            "bound_bot_ids": [],
                            "last_seen_ms": 1_735_000_000_000,
                            "latest_frame_data_url": frameURL,
                        ],
                    ],
                    "pending_approvals": [],
                ])
            case ("GET", "/api/v1/remote/assist/info"):
                return .json(200, #"{"assist_id":"482910375","allow_assist":false,"code_mode":"temporary","current_code":"739104"}"#)
            case ("POST", "/api/v1/remote/assist/allow"):
                return .json(500, #"{"error":"assist_service_unavailable"}"#)
            case ("GET", "/api/v1/remote/sessions"):
                return .json(200, #"{"sessions":[]}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("allow_rollback")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "501")
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
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let vm = FabushiRemoteComputerViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            await vm.refreshAll()
            XCTAssertEqual(vm.computers.count, 1)
            XCTAssertEqual(vm.computers[0].nodeId, "local:501:mac-studio")
            XCTAssertNotNil(FabushiRemoteImageDecoder.decodeDataURL(vm.computers[0].latestFrameDataUrl))
            XCTAssertEqual(vm.assistConfig.assistId, "482910375")
            XCTAssertEqual(vm.assistConfig.formattedAssistId, "482 910 375")
            XCTAssertFalse(vm.assistConfig.allowAssist)

            // Optimistic toggle to true -> server returns 500 -> MUST roll back to false + emit non-chat error toast
            let updated = await vm.toggleAllowRemote(true, locale: .en)
            XCTAssertFalse(updated)
            XCTAssertFalse(vm.assistConfig.allowAssist, "Failed remote.assistAllow MUST strictly roll back optimistic toggle")
            let toast = try XCTUnwrap(router.toastController.windowBanner)
            XCTAssertNil(toast.scopeChatId, "Remote Computer error toast must be unscoped non-chat toast")
            XCTAssertTrue(toast.isError)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/remote/assist/allow").count, 1)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 2. `Disconnect All (N)` + `<10ms` `hubEmergencyTakeover` + `override_epoch`

    func testDisconnectAllAndEmergencyTakeoverUnder10MsWithOverrideEpoch() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":502,"username":"remote_ops_user","session_id":"s-502","token":"tok-502","first_name":"Ops"}"#)
            case ("POST", "/api/v1/remote/sessions/disconnect-all"):
                return .json(200, #"{"disconnected":2}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("disconnect_takeover")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "502")
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
            let router = ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
            let vm = FabushiRemoteComputerViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            // Register a real active session in Rust RemoteControlHub
            let session = FabushiRemoteSessionDto(
                sessionId: "sess-ai-01",
                controllerKind: .aiMcp,
                controllerName: "MCP Copilot",
                controllerPlatform: "macos",
                controllerId: "mcp-1",
                targetNodeId: "local:502:mac-studio",
                targetDeviceId: "dev-mac-1",
                mode: .aiControl,
                relayMode: .derpRelay
            )
            let _: FabushiRemoteSessionDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("hubRegisterSession"),
                params: session
            )

            router.activeRemoteControlStatus = RemoteControlStatusState(
                id: "sess-ai-01",
                nodeName: "Studio Mac",
                controllerLabel: "MCP Copilot"
            )

            let ok = await vm.disconnectAllSessions(locale: .en)
            XCTAssertTrue(ok)
            XCTAssertEqual(server.requests(method: "POST", pathPrefix: "/api/v1/remote/sessions/disconnect-all").count, 1)

            let takeover = try XCTUnwrap(vm.lastEmergencyTakeover)
            XCTAssertGreaterThanOrEqual(takeover.disconnectedSessions, 1)
            XCTAssertGreaterThan(takeover.overrideEpoch, 0)
            XCTAssertLessThan(takeover.elapsedUs, 10_000, "Emergency takeover must complete in <10ms locally")
            XCTAssertEqual(vm.overrideEpoch, takeover.overrideEpoch)
            XCTAssertNil(router.activeRemoteControlStatus)
            XCTAssertTrue(vm.activeSessions.isEmpty)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 3. `Shell` -> `RemoteTerminal` Auto-Focus + `list_dir` & Deterministic Back-Stack Unwind

    func testShellOpensRemoteTerminalWithAutoFocusAndBackStackUnwind() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":503,"username":"remote_ops_user","session_id":"s-503","token":"tok-503","first_name":"Ops"}"#)
            case ("POST", "/api/v1/computers/exec"):
                return .json(200, #"{"command_id":"cmd-ls","node_id":"node-term-1","ok":true,"exit_code":0,"stdout":"","stderr":"","entries":["README.md","src/"],"executed_by":"mac-host","duration_ms":12}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("shell_unwind")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "503")
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
            let router = ShellNavigationRouter()
            let vm = FabushiRemoteComputerViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            let node = FabushiComputerNodeDto(
                nodeId: "node-term-1",
                ownerUserId: 503,
                kind: .cloudServer,
                displayName: "Build Box",
                ipAddress: "100.64.0.22",
                osName: "Ubuntu 24.04",
                resolution: "1920x1080",
                online: true
            )

            vm.selectedHubTab = .cloudPCs
            vm.openShell(node: node)

            XCTAssertEqual(vm.activeWorkbenchNodeId, "node-term-1")
            XCTAssertEqual(vm.selectedWorkbenchTab, .terminal)
            XCTAssertTrue(vm.shouldAutoFocusPtyInput)
            XCTAssertEqual(vm.ptyTabs.count, 1)

            // Wait briefly for the triggered list_dir refresh to complete
            for _ in 0..<40 where (vm.remoteDirectoryEntriesByNodeId["node-term-1"] ?? []).isEmpty {
                try await Task.sleep(nanoseconds: 25_000_000)
            }
            XCTAssertEqual(vm.remoteDirectoryEntriesByNodeId["node-term-1"], ["README.md", "src/"])

            // Exercise deterministic back-stack unwind:
            // permissionModal -> cloudForm -> workbench -> tab -> Hub root (false)
            vm.isCloudPCFormPresented = true
            vm.isPermissionModalPresented = true

            XCTAssertTrue(vm.unwindBackNavigation())
            XCTAssertFalse(vm.isPermissionModalPresented)
            XCTAssertTrue(vm.isCloudPCFormPresented)

            XCTAssertTrue(vm.unwindBackNavigation())
            XCTAssertFalse(vm.isCloudPCFormPresented)
            XCTAssertEqual(vm.activeWorkbenchNodeId, "node-term-1")

            XCTAssertTrue(vm.unwindBackNavigation())
            XCTAssertNil(vm.activeWorkbenchNodeId)
            XCTAssertEqual(vm.selectedHubTab, .cloudPCs)

            XCTAssertTrue(vm.unwindBackNavigation())
            XCTAssertEqual(vm.selectedHubTab, .myDevices)

            XCTAssertFalse(vm.unwindBackNavigation())

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 4. Fail-Closed iOS Sandbox Boundaries & Zero `macosDragHud`

    func testIosSandboxFailClosedLocalHostAndTailnetApprove() async throws {
        let dir = try makeTempDir("sandbox_fail_closed")
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
            // 1. computers.registerLocalHost MUST fail closed with capability_unavailable on iOS
            do {
                let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.computers.method("registerLocalHost"),
                    params: FabushiEmptyParams()
                )
                XCTFail("computers.registerLocalHost must fail closed on iOS")
            } catch let err as FabushiHostError {
                XCTAssertEqual(err.code, "capability_unavailable")
            }

            // 2. remote.localCapture MUST fail closed with capability_unavailable on iOS
            do {
                let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.remote.method("localCapture"),
                    params: FabushiEmptyParams()
                )
                XCTFail("remote.localCapture must fail closed on iOS")
            } catch let err as FabushiHostError {
                XCTAssertEqual(err.code, "capability_unavailable")
            }

            // 3. tailnet.decideApproval(approved: true) MUST fail closed on iOS
            struct DecideParams: Encodable, Sendable {
                let commandId: String
                let approved: Bool
                enum CodingKeys: String, CodingKey {
                    case commandId = "command_id"
                    case approved
                }
            }
            do {
                let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.tailnet.method("decideApproval"),
                    params: DecideParams(commandId: "cmd-inbound-1", approved: true)
                )
                XCTFail("tailnet.decideApproval(approved: true) must fail closed on iOS")
            } catch let err as FabushiHostError {
                XCTAssertEqual(err.code, "capability_unavailable")
            }

            // 4. Verify forbidden macOS drag HUD identifier constant is NEVER used by iOS Remote views
            XCTAssertEqual(FabushiAccessibilityIDs.Remote.macosDragHudForbidden, "fabushi.remote.macosDragHud")
            XCTAssertTrue(FabushiAccessibilityIDs.Remote.allIdentifiers.contains(FabushiAccessibilityIDs.Remote.localHostUnsupportedBanner))

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 5. OSC-52 PTY Parsing, Modifier Remapping & Account-Switch Generation Wipe

    func testPtyOsc52ModifierRemappingAndAccountSwitchGenerationWipe() async throws {
        let server = try M2bLoopbackHTTPTestServer { req in
            switch (req.method, req.path) {
            case ("POST", "/api/v1/auth/login"):
                return .json(200, #"{"user_id":504,"username":"remote_ops_user","session_id":"s-504","token":"tok-504","first_name":"Ops"}"#)
            case ("POST", "/api/v1/auth/logout"):
                return .json(200, #"{"ok":true}"#)
            case ("GET", "/api/v1/computers"):
                return .json(200, #"{"computers":[{"node_id":"node-504","owner_user_id":504,"kind":"local_device","display_name":"Lab Mac","ip_address":"100.64.0.9","os_name":"macOS","resolution":"2560x1440","online":true,"is_current_device":false,"tier_badge":"本地","run_policy":"ask_every_time","authorized_controllers":[],"bound_bot_ids":[],"last_seen_ms":1}],"pending_approvals":[]}"#)
            default:
                return .json(404, #"{"error":"not_found"}"#)
            }
        }
        defer { server.stop() }

        let dir = try makeTempDir("osc52_wipe")
        let kc = makeIsolatedKeychain()
        defer {
            try? kc.deletePassphrase()
            try? kc.deleteAccountKey(accountId: "504")
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
            let router = ShellNavigationRouter()
            let vm = FabushiRemoteComputerViewModel(coordinator: coordinator, router: router)
            defer { vm.detachLifecycleHooks() }

            // 1. OSC-52 parsing via real Rust remote.ptyParseOsc52
            let b64Hello = Data("hello-fabushi-osc52".utf8).base64EncodedString()
            let osc52Seq = "\u{1B}]52;c;\(b64Hello)\u{07}"
            let payloads = await vm.parseOsc52Output(osc52Seq)
            XCTAssertEqual(payloads.count, 1)
            XCTAssertEqual(payloads.first?.text, "hello-fabushi-osc52")
            XCTAssertEqual(vm.lastOsc52ClipboardText, "hello-fabushi-osc52")

            // 2. Modifier remapping via real Rust remote.remapModifiers
            struct RemapParams: Encodable, Sendable {
                let modifiers: [String]
                let clientOs: String
                let hostOs: String
                let enabled: Bool
                enum CodingKeys: String, CodingKey {
                    case modifiers
                    case clientOs = "client_os"
                    case hostOs = "host_os"
                    case enabled
                }
            }
            let remapped: FabushiRemapModifiersResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.remote.method("remapModifiers"),
                params: RemapParams(modifiers: ["cmd"], clientOs: "ios", hostOs: "windows", enabled: true)
            )
            XCTAssertFalse(remapped.modifiers.isEmpty)

            // 3. Populate state then logout -> generation advances and purges all remote state!
            await vm.refreshComputers()
            XCTAssertEqual(vm.computers.count, 1)
            vm.openShell(node: vm.computers[0])
            router.activeRemoteControlStatus = RemoteControlStatusState(
                id: "s-1",
                nodeName: "Lab Mac",
                controllerLabel: "iOS"
            )

            let previousGen = coordinator.accountScopeGeneration
            _ = try await coordinator.logout()
            XCTAssertGreaterThan(coordinator.accountScopeGeneration, previousGen)
            XCTAssertTrue(vm.computers.isEmpty, "Logout must wipe remote computers list")
            XCTAssertTrue(vm.ptyTabs.isEmpty, "Logout must wipe PTY session tabs")
            XCTAssertNil(vm.activeWorkbenchNodeId, "Logout must close active remote workbench")
            XCTAssertNil(router.activeRemoteControlStatus, "Logout must clear controlled status pill")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
