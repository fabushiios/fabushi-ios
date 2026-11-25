import Foundation
import SwiftUI
import UIKit
import XCTest
@testable import Fabushi

@MainActor
final class FabushiMultiAgentProtocolTests: XCTestCase, @unchecked Sendable {
    private func makeTempDir(_ suffix: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi_m4a_multiagent_\(suffix)_\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func makeIsolatedKeychain(_ suffix: String) -> FabushiKeychainStore {
        FabushiKeychainStore(
            service: "com.ombhrum.fabushi.tests.m4a.multiagent.\(suffix).\(UUID().uuidString)",
            account: "m4a-ma-\(suffix)"
        )
    }

    private func makeRouter() -> ShellNavigationRouter {
        ShellNavigationRouter(toastController: FabushiToastController(autoDismissEnabled: false))
    }

    // MARK: - 1. FloorControlQueue (`D14-02` 120s Lease & `UserInterruption` Preempt), 5-Hop LoopBudget & Workspace CAS

    func testRealHostFloorControl120sLeaseLoopBudgetAndWorkspaceCASConflict() async throws {
        let dir = try makeTempDir("floor_loop_workspace")
        defer { try? FileManager.default.removeItem(at: dir) }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: makeIsolatedKeychain("flw"))
        let router = makeRouter()
        let hub = FabushiAgentStudioLogoutCleanupHub()
        let chatId: Int64 = -100991122
        let vm = FabushiMultiAgentViewModel(
            chatId: chatId,
            coordinator: coordinator,
            router: router,
            cleanupHub: hub
        )

        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )
        do {
            // 1. FloorControlQueue: explicit 120s group floor lease (`D14-02`)
            let req1Raw = await vm.requestFloor(
                agentId: "rust_bot",
                priority: .directMention,
                leaseMs: FabushiMultiAgentConstants.groupFloorLeaseMs
            )
            let req1 = try XCTUnwrap(req1Raw)
            XCTAssertEqual(req1.granted, true)
            XCTAssertEqual(req1.currentSpeaker, "rust_bot")
            XCTAssertEqual(req1.activeGrant?.staleAfterMs, 120_000)

            await vm.loadFloorStatus()
            XCTAssertEqual(vm.floorStatus.activeGrant?.staleAfterMs, 120_000)
            if let grant = vm.floorStatus.activeGrant {
                let remaining = grant.remainingLeaseSeconds(atMs: grant.acquiredAtMs + 5_000)
                XCTAssertEqual(remaining, 115)
            }

            // Second agent enqueues behind active speaker
            let req2Raw = await vm.requestFloor(agentId: "qa_bot", priority: .generalContribution)
            let req2 = try XCTUnwrap(req2Raw)
            XCTAssertEqual(req2.granted, false)
            XCTAssertEqual(req2.queuedCount, 1)

            // UserInterruption preempts active speaker and emits a `trn_` nonce
            let preemptRaw = await vm.preemptFloorForUser()
            let preempt = try XCTUnwrap(preemptRaw)
            XCTAssertEqual(preempt.preemptedAgent, "rust_bot")
            XCTAssertTrue(preempt.preemptedNonce?.hasPrefix("trn_") ?? false)
            XCTAssertNil(preempt.currentSpeaker)

            // Re-acquire and release cleanly
            _ = await vm.requestFloor(agentId: "qa_bot", priority: .taskAssignee)
            let releasedRaw = await vm.releaseFloor(agentId: "qa_bot")
            let released = try XCTUnwrap(releasedRaw)
            XCTAssertNil(released.currentSpeaker)

            // 2. 5-Hop LoopBudget: 5 hops succeed, 6th hop fails closed (`loop_budget_exceeded`), reset clears trace
            for hopIdx in 1...5 {
                let statusRaw = await vm.recordLoopHop(agentId: "agent_\(hopIdx)")
                let status = try XCTUnwrap(statusRaw)
                XCTAssertEqual(status.currentHop, UInt32(hopIdx))
                XCTAssertEqual(status.maxHops, 5)
                XCTAssertEqual(status.exhausted, hopIdx == 5)
            }
            let sixthHop = await vm.recordLoopHop(agentId: "agent_6")
            XCTAssertNil(sixthHop, "6th hop must fail closed with loop_budget_exceeded")
            XCTAssertEqual(vm.loopBudget.exhausted, true)

            let resetOk = await vm.resetLoopBudget()
            XCTAssertTrue(resetOk)
            XCTAssertEqual(vm.loopBudget.currentHop, 0)
            XCTAssertTrue(vm.loopBudget.trace.isEmpty)

            // 3. Revision-Fenced SharedGroupWorkspace: CAS conflict detection + overwrite + remove -> nil
            let set1Raw = await vm.setWorkspaceEntry(
                key: "plan.md",
                value: .string("# Initial Plan"),
                updatedBy: "alice",
                expectedRevision: 0
            )
            let set1 = try XCTUnwrap(set1Raw)
            XCTAssertEqual(set1.revision, 1)

            // Stale expected_revision (0 when actual is 1) MUST trigger `.conflict`!
            let conflictAttempt = await vm.setWorkspaceEntry(
                key: "plan.md",
                value: .string("# Stale Update"),
                updatedBy: "bob",
                expectedRevision: 0
            )
            XCTAssertNil(conflictAttempt)
            XCTAssertNotNil(vm.workspaceConflict)
            XCTAssertEqual(vm.workspaceConflict?.expectedRevision, 0)

            // Overwrite conflict reloads latest rev 1 and commits rev 2
            let resolved = await vm.overwriteWorkspaceConflict()
            XCTAssertTrue(resolved)
            XCTAssertNil(vm.workspaceConflict)
            XCTAssertEqual(vm.workspace.revision, 2)

            let fetchedEntry = await vm.getWorkspaceEntry(key: "plan.md")
            XCTAssertEqual(fetchedEntry?.displayValueText, "# Stale Update")

            let removed = await vm.removeWorkspaceEntry(key: "plan.md")
            XCTAssertEqual(removed?.key, "plan.md")
            let afterRemove = await vm.getWorkspaceEntry(key: "plan.md")
            XCTAssertNil(afterRemove, "workspaceGet must decode JSON null as nil after removal")

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }

    // MARK: - 2. `D14-01` Wake Gate, Stay-Silent `[PASS]`, Ed25519 Capability Tokens & Logout Isolation

    func testRealHostWakeGateD1401OutboundReviewCapabilityTokensAndLogoutReset() async throws {
        let dir = try makeTempDir("wake_pass_cap_logout")
        defer { try? FileManager.default.removeItem(at: dir) }

        let coordinator = FabushiCoordinator(host: FabushiRustHostActor(), keychain: makeIsolatedKeychain("wake"))
        let router = makeRouter()
        let hub = FabushiAgentStudioLogoutCleanupHub()
        let chatId: Int64 = -100887766
        let vm = FabushiMultiAgentViewModel(
            chatId: chatId,
            coordinator: coordinator,
            router: router,
            cleanupHub: hub
        )

        let candidates = [
            FabushiMultiAgentWakeBotCandidateDTO(
                botUserId: FabushiMultiAgentConstants.officialDefaultBotUserId,
                agentId: "fabushi_default",
                name: "Fabushi Generalist",
                username: "fabushi_bot",
                role: "assistant",
                skills: ["general"]
            ),
            FabushiMultiAgentWakeBotCandidateDTO(
                botUserId: 900_002,
                agentId: "rust_bot",
                name: "Rust Architect",
                username: "rust_bot",
                role: "compiler engineer",
                skills: ["rust", "ffi"]
            ),
        ]

        try await coordinator.start(
            appDataDirectory: dir,
            serverURL: "http://127.0.0.1:9",
            testMode: true,
            startNetworkMonitor: false
        )
        do {
            _ = try await coordinator.setAuthSession(FabushiSessionAuthParams(
                userId: 505,
                username: "multi_agent_tester",
                sessionId: "sess-505",
                token: "tok-505"
            ))

            // 1. CJK-adjacent ASCII mention (`@rust_bot你好`)
            let cjkDecisionRaw = await vm.evaluateWakeGate(
                FabushiMultiAgentWakeGateInputDTO(candidates: candidates, messageText: "@rust_bot你好 请看一下")
            )
            let cjkDecision = try XCTUnwrap(cjkDecisionRaw)
            XCTAssertTrue(cjkDecision.shouldWake)
            XCTAssertEqual(cjkDecision.reason, .explicitMention)
            XCTAssertEqual(cjkDecision.wakeBotUserIds, [900_002])

            // 2. Slash command bot mention (`/status@rust_bot`)
            let slashDecisionRaw = await vm.evaluateWakeGate(
                FabushiMultiAgentWakeGateInputDTO(candidates: candidates, messageText: "/status@rust_bot")
            )
            let slashDecision = try XCTUnwrap(slashDecisionRaw)
            XCTAssertTrue(slashDecision.shouldWake)
            XCTAssertEqual(slashDecision.reason, .explicitMention)
            XCTAssertEqual(slashDecision.wakeBotUserIds, [900_002])

            // 3. `@所有人你好` wakes all bots
            let mentionAllDecisionRaw = await vm.evaluateWakeGate(
                FabushiMultiAgentWakeGateInputDTO(candidates: candidates, messageText: "@所有人你好 开会了")
            )
            let mentionAllDecision = try XCTUnwrap(mentionAllDecisionRaw)
            XCTAssertTrue(mentionAllDecision.shouldWake)
            XCTAssertEqual(mentionAllDecision.reason, .mentionAll)
            XCTAssertEqual(Set(mentionAllDecision.wakeBotUserIds), Set([FabushiMultiAgentConstants.officialDefaultBotUserId, 900_002]))

            // 4. Display-name mention (`@Rust Architect`)
            let displayNameDecisionRaw = await vm.evaluateWakeGate(
                FabushiMultiAgentWakeGateInputDTO(candidates: candidates, messageText: "@Rust Architect please inspect")
            )
            let displayNameDecision = try XCTUnwrap(displayNameDecisionRaw)
            XCTAssertTrue(displayNameDecision.shouldWake)
            XCTAssertEqual(displayNameDecision.reason, .explicitMention)
            XCTAssertEqual(displayNameDecision.wakeBotUserIds, [900_002])

            // 5. Low-signal ack suppression (`收到`, `ok thanks`)
            for ack in ["收到", "ok thanks"] {
                let ackDecisionRaw = await vm.evaluateWakeGate(
                    FabushiMultiAgentWakeGateInputDTO(candidates: candidates, messageText: ack)
                )
                let ackDecision = try XCTUnwrap(ackDecisionRaw)
                XCTAssertFalse(ackDecision.shouldWake, "Expected low-signal ack '\(ack)' to stay silent")
                XCTAssertEqual(ackDecision.outcome, .pass)
                XCTAssertEqual(ackDecision.reason, .noRelevantCandidate)
            }

            // 6. Unmentioned human question falls back deterministically to `primary_fallback` (`v1.6.14`)
            let fallbackDecisionRaw = await vm.evaluateWakeGate(
                FabushiMultiAgentWakeGateInputDTO(
                    candidates: candidates,
                    messageText: "今天下午三点天气怎么样",
                    senderIsHuman: true
                )
            )
            let fallbackDecision = try XCTUnwrap(fallbackDecisionRaw)
            XCTAssertTrue(fallbackDecision.shouldWake)
            XCTAssertEqual(fallbackDecision.reason, .primaryFallback)
            XCTAssertEqual(fallbackDecision.wakeBotUserIds, [FabushiMultiAgentConstants.officialDefaultBotUserId])

            // 7. Stage 2 Outbound Review & Stay-Silent `[PASS]` sentinels + duplicate echo
            for sentinel in ["[PASS]", "<PASS/>", "NO_REPLY"] {
                let isPassRaw = await vm.checkIsPassSentinel(sentinel)
                let isPass = try XCTUnwrap(isPassRaw)
                XCTAssertTrue(isPass, "Expected '\(sentinel)' to be recognized as pass sentinel")

                let verdictRaw = await vm.evaluateOutboundReview(
                    FabushiMultiAgentOutboundReviewInputDTO(agentId: "rust_bot", candidateText: sentinel)
                )
                let verdict = try XCTUnwrap(verdictRaw)
                XCTAssertFalse(verdict.shouldSend)
                XCTAssertEqual(verdict.outcome, .pass)
                XCTAssertEqual(verdict.reason, .explicitPassSentinel)
            }

            let dupVerdictRaw = await vm.evaluateOutboundReview(
                FabushiMultiAgentOutboundReviewInputDTO(
                    agentId: "rust_bot",
                    candidateText: "The build succeeded with zero warnings on iOS 17.",
                    recentRoomMessages: ["The build succeeded with zero warnings on iOS 17."]
                )
            )
            let dupVerdict = try XCTUnwrap(dupVerdictRaw)
            XCTAssertFalse(dupVerdict.shouldSend)
            XCTAssertEqual(dupVerdict.reason, .duplicateEcho)

            let okVerdictRaw = await vm.evaluateOutboundReview(
                FabushiMultiAgentOutboundReviewInputDTO(
                    agentId: "rust_bot",
                    candidateText: "I checked the Cargo lockfile and verified all crates compile."
                )
            )
            let okVerdict = try XCTUnwrap(okVerdictRaw)
            XCTAssertTrue(okVerdict.shouldSend)
            XCTAssertEqual(okVerdict.reason, .approved)

            // 8. Scoped Ed25519 Capability Token issue & verify
            let issuedRaw = await vm.issueCapabilityToken(
                agentId: "rust_bot",
                allowedActions: ["workspace.write", "floor.release"],
                ttlSecs: 300
            )
            let issued = try XCTUnwrap(issuedRaw)
            XCTAssertEqual(issued.verifyingKeyHex.count, 64)

            let validVerifyRaw = await vm.verifyCapabilityToken(
                token: issued.token,
                verifyingKeyHex: issued.verifyingKeyHex,
                agentId: "rust_bot",
                action: "workspace.write"
            )
            let validVerify = try XCTUnwrap(validVerifyRaw)
            XCTAssertTrue(validVerify.valid)
            XCTAssertEqual(validVerify.permitsAction, true)

            let deniedVerifyRaw = await vm.verifyCapabilityToken(
                token: issued.token,
                verifyingKeyHex: issued.verifyingKeyHex,
                agentId: "rust_bot",
                action: "terminal.exec"
            )
            let deniedVerify = try XCTUnwrap(deniedVerifyRaw)
            XCTAssertFalse(deniedVerify.valid)
            XCTAssertEqual(deniedVerify.permitsAction, false)

            // 9. Logout / Generation Isolation wipes all Multi-Agent ViewModel state immediately
            _ = await vm.requestFloor(agentId: "rust_bot")
            _ = try await coordinator.clearLocalSession()
            XCTAssertNil(vm.floorStatus.currentSpeaker)
            XCTAssertNil(vm.lastWakeDecision)
            XCTAssertNil(vm.lastOutboundVerdict)
            XCTAssertNil(vm.lastIssuedToken)
            XCTAssertTrue(vm.workspace.entries.isEmpty)

            // 10. Multi-Agent Room Accessories View builds cleanly
            let banner = FabushiMultiAgentRoomAccessoriesView(
                chatId: chatId,
                coordinator: coordinator,
                router: router,
                cleanupHub: hub
            )
            let hosting = UIHostingController(rootView: banner.environment(FabushiTheme.default))
            XCTAssertNotNil(hosting.view)

            await coordinator.shutdown()
        } catch {
            await coordinator.shutdown()
            throw error
        }
    }
}
