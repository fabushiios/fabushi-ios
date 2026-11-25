import Foundation
import SwiftUI
import XCTest
@testable import Fabushi

@MainActor
final class FabushiAgentStudioModuleWiringTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m4b_module_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain(_ suffix: String) -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4b.module.\(suffix).\(UUID().uuidString)",
            account: "m4b-module-\(suffix)"
        )
    }

    private func makeRouter() -> ShellNavigationRouter {
        ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
    }

    // MARK: - 1. Sealed `AgentStudioModuleProviding` Conformance, 1-Line Registry Wiring & All 5 Factories

    func testAgentStudioModuleConformanceIsolatedRegistryAndAllFiveFactories() async throws {
        let dir = try makeTempDir("factories")
        let kc = makeIsolatedKeychain("factories")
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

        let router = makeRouter()
        // Strict zero-argument initializer required for 1-line M6 registration
        let module = FabushiAgentStudioModule()
        let registry = FabushiFeatureModuleRegistry()
        registry.register(agentStudio: module)

        XCTAssertTrue(registry.agentStudio is FabushiAgentStudioModule)

        // Invoke ALL 5 sealed `AgentStudioModuleProviding` factory methods
        let workspaceView = registry.agentStudio.makeAgentStudioWorkspaceView(
            initialTab: "skills",
            coordinator: coordinator,
            router: router
        )
        let magpieView = registry.agentStudio.makeMagpieAiProvidersSettingsView(
            coordinator: coordinator,
            router: router
        )
        let mcpView = registry.agentStudio.makeOfficialMcpConsoleView(
            coordinator: coordinator,
            router: router
        )
        let codingAccessories = registry.agentStudio.makeCodingAgentThreadAccessories(
            chatId: 8001,
            coordinator: coordinator,
            router: router
        )
        let multiAgentAccessories = registry.agentStudio.makeMultiAgentRoomAccessories(
            chatId: 8001,
            coordinator: coordinator,
            router: router
        )

        // Verify all 5 views construct valid AnyView instances
        XCTAssertEqual(String(describing: type(of: workspaceView)), "AnyView")
        XCTAssertEqual(String(describing: type(of: magpieView)), "AnyView")
        XCTAssertEqual(String(describing: type(of: mcpView)), "AnyView")
        XCTAssertEqual(String(describing: type(of: codingAccessories)), "AnyView")
        XCTAssertEqual(String(describing: type(of: multiAgentAccessories)), "AnyView")

        await coordinator.shutdown()
    }

    // MARK: - 2. `installLogoutCleanup` Purge Propagation on Logout & Server Switch Across All 5 M4 ViewModels

    func testLogoutAndServerSwitchCleanupPropagatesAcrossAllM4ViewModels() async throws {
        let dir = try makeTempDir("logout_propagation")
        let kc = makeIsolatedKeychain("logout_propagation")
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
            let router = makeRouter()
            let module = FabushiAgentStudioModule()
            module.installLogoutCleanup(on: coordinator)

            let studioVM = FabushiAgentStudioViewModel(
                coordinator: coordinator,
                router: router,
                cleanupHub: module.logoutCleanupHub
            )
            let multiAgentVM = FabushiMultiAgentViewModel(
                chatId: 700,
                coordinator: coordinator,
                router: router,
                cleanupHub: module.logoutCleanupHub
            )
            let magpieVM = FabushiMagpieViewModel(
                coordinator: coordinator,
                router: router,
                cleanupHub: module.logoutCleanupHub
            )
            let codingVM = FabushiCodingViewModel(
                chatId: 700,
                coordinator: coordinator,
                router: router,
                cleanupHub: module.logoutCleanupHub
            )
            let mcpVM = FabushiMcpViewModel(
                coordinator: coordinator,
                router: router,
                cleanupHub: module.logoutCleanupHub
            )
            XCTAssertEqual(module.logoutCleanupHub.registeredHandlerCount, 5)

            // Sign in and seed live state on all 5 ViewModels
            _ = try await coordinator.setAuthSession(
                FabushiSessionAuthParams(
                    userId: 909,
                    username: "full_m4_user",
                    sessionId: "sess-909",
                    token: "tok-909",
                    firstName: "M4"
                )
            )
            _ = await studioVM.createTask(taskId: "task_before_logout")
            _ = await multiAgentVM.requestFloor(agentId: "bot_a")
            magpieVM.activeProvider = "magpie_provider:google-vertex"
            magpieVM.activeModel = "gemini-2.5-pro"
            magpieVM.importLinkDraft = "magpie://import?preset=openai&key=sk-temp"
            codingVM.botFatherPromptDraft = "Build a Swift coding bot"
            codingVM.streamingDiffPreview = "+let secret = 1"
            mcpVM.pairTokenInput = "pair_live_secret"

            // Trigger real coordinator.logout() -> purger fires automatically!
            _ = try? await coordinator.logout()

            XCTAssertTrue(studioVM.tasks.isEmpty)
            XCTAssertNil(multiAgentVM.floorStatus.currentSpeaker)
            XCTAssertEqual(magpieVM.activeProvider, "")
            XCTAssertEqual(magpieVM.activeModel, "")
            XCTAssertEqual(magpieVM.importLinkDraft, "")
            XCTAssertEqual(codingVM.botFatherPromptDraft, "")
            XCTAssertEqual(codingVM.streamingDiffPreview, "")
            XCTAssertEqual(mcpVM.pairTokenInput, "")

            // Re-seed state and trigger real server switch -> purger fires automatically again!
            magpieVM.probeStatusText = "server A probe"
            codingVM.lastRemoteHostProbeSummary = "server A host"
            mcpVM.pairTokenInput = "pair_server_a"
            _ = try await coordinator.setServerURL("http://127.0.0.1:19")

            XCTAssertEqual(magpieVM.probeStatusText, "")
            XCTAssertEqual(codingVM.lastRemoteHostProbeSummary, "")
            XCTAssertEqual(mcpVM.pairTokenInput, "")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 3. Complete M4a + M4b Bilingual String Parity Check

    func testAllM4aAndM4bBilingualStringsParityAndForbiddenPrefixes() throws {
        let hanRegex = try NSRegularExpression(pattern: "\\p{Han}")
        let asciiLetterRegex = try NSRegularExpression(pattern: "[A-Za-z]")
        let keys = FabushiAgentStudioStrings.allKeys

        XCTAssertGreaterThanOrEqual(keys.count, 200)
        XCTAssertTrue(keys.contains("m4.magpie.probePendingM6Notice"))
        XCTAssertTrue(keys.contains("m4.coding.accountIsolationBadge"))
        XCTAssertTrue(keys.contains("m4.mcp.officialCatalogFormat"))

        for key in keys {
            XCTAssertTrue(key.hasPrefix("m4."))
            let en = FabushiAgentStudioStrings.text(key, locale: .en)
            let zh = FabushiAgentStudioStrings.text(key, locale: .zhHans)

            XCTAssertFalse(en.isEmpty, "Empty en for \(key)")
            XCTAssertFalse(zh.isEmpty, "Empty zhHans for \(key)")
            XCTAssertNotEqual(en, zh, "en and zhHans must differ for \(key)")

            let enRange = NSRange(en.startIndex..<en.endIndex, in: en)
            let zhRange = NSRange(zh.startIndex..<zh.endIndex, in: zh)
            XCTAssertNotNil(asciiLetterRegex.firstMatch(in: en, range: enRange), "Missing ASCII in en for \(key): \(en)")
            XCTAssertNil(hanRegex.firstMatch(in: en, range: enRange), "Unexpected Han in en for \(key): \(en)")
            XCTAssertNotNil(hanRegex.firstMatch(in: zh, range: zhRange), "Missing Han in zhHans for \(key): \(zh)")

            for value in [en, zh] {
                for prefix in FabushiAccessibilityIDs.forbiddenRawLocalizationKeyPrefixes {
                    XCTAssertFalse(
                        value.lowercased().contains(prefix),
                        "Forbidden prefix '\(prefix)' found in '\(value)' for key \(key)"
                    )
                }
            }
        }
    }
}
