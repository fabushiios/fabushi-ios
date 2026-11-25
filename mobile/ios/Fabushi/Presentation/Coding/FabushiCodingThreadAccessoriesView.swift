import Foundation
import SwiftUI

/// Lightweight self-sizing inline stage accessories banner for `makeCodingAgentThreadAccessories(chatId:...)`.
///
/// Strictly ZERO nested root `NavigationStack` and ZERO full-height `List`.
/// Mounts `Stage.codingDiffCard`, `Stage.codingCheckpointButton`, `Inspector.codingHostProbeButton`,
/// `Studio.codingAgentCreatePRButton`, and `Stage.toolApprovalDock` / `toolApproveButton` / `toolDenyButton`,
/// with modal detail sheets (wrapped in `NavigationStack`) for unified diff, worktree/checkpoints, and PRs.
@MainActor
public struct FabushiCodingThreadAccessoriesView: View {
    public let chatId: Int64
    public let botUserId: Int64

    @State private var viewModel: FabushiCodingViewModel
    @State private var isDiffSheetPresented: Bool = false
    @State private var isCheckpointsSheetPresented: Bool = false
    @State private var isPullRequestSheetPresented: Bool = false
    @State private var prTitleDraft: String = ""
    @State private var checkpointBeforeRef: String = "HEAD~1"
    @State private var checkpointAfterRef: String = "HEAD"

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        chatId: Int64,
        botUserId: Int64 = 0,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self.chatId = chatId
        self.botUserId = botUserId
        self._viewModel = State(
            initialValue: FabushiCodingViewModel(
                chatId: chatId,
                coordinator: coordinator,
                router: router,
                cleanupHub: cleanupHub
            )
        )
    }

    public var body: some View {
        let locale = theme.locale
        let isCompact = horizontalSizeClass == .compact

        VStack(alignment: .leading, spacing: 8) {
            // Streaming Diff & Worktree Card (`fabushi.stage.codingDiffCard`)
            Button {
                isDiffSheetPresented = true
            } label: {
                VStack(alignment: .leading, spacing: 6) {
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 8) {
                            diffIdentityCluster(locale: locale)
                            Spacer(minLength: 4)
                            diffMetricPill(locale: locale)
                        }
                        VStack(alignment: .leading, spacing: 6) {
                            diffIdentityCluster(locale: locale)
                            diffMetricPill(locale: locale)
                        }
                    }

                    if let branch = viewModel.activeThread?.branchName, !branch.isEmpty {
                        HStack(spacing: 6) {
                            Image(systemName: "arrow.branch")
                                .accessibilityHidden(true)
                            Text("\(FabushiAgentStudioStrings.text("m4.coding.worktreeLabel", locale: locale)): \(branch)")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(1)
                        }
                    }
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fabushiMinimumTouchTarget()
                .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.codingDiffCard)

            // Action Strip: Checkpoint + Probe Host + Create PR
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    checkpointButton(locale: locale)
                    probeHostButton(locale: locale)
                    createPrButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        checkpointButton(locale: locale)
                        probeHostButton(locale: locale)
                    }
                    createPrButton(locale: locale)
                }
            }

            // Interactive Value-Bound Coding Approval Dock (`fabushi.stage.toolApprovalDock`)
            if let prompt = viewModel.pendingApprovals.first {
                codingApprovalDock(prompt: prompt, locale: locale, isCompact: isCompact)
            }
        }
        .padding(12)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
        )
        .task(id: "\(chatId):\(viewModel.coordinator.accountScopeGeneration)") {
            await viewModel.loadAll(botUserId: botUserId != 0 ? botUserId : nil)
        }
        .onChange(of: viewModel.coordinator.accountScopeGeneration) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .onChange(of: viewModel.coordinator.activeAccountId) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .sheet(isPresented: $isDiffSheetPresented) {
            diffDetailSheet(locale: locale, isCompact: isCompact)
                .environment(theme)
        }
        .sheet(isPresented: $isCheckpointsSheetPresented) {
            checkpointsSheet(locale: locale, isCompact: isCompact)
                .environment(theme)
        }
        .sheet(isPresented: $isPullRequestSheetPresented) {
            pullRequestSheet(locale: locale, isCompact: isCompact)
                .environment(theme)
        }
    }

    // MARK: - Subviews

    @ViewBuilder
    private func diffIdentityCluster(locale: FabushiLocale) -> some View {
        HStack(spacing: 6) {
            Image(systemName: "doc.text.magnifyingglass")
                .foregroundStyle(theme.accentColor)
                .accessibilityHidden(true)
            Text(FabushiAgentStudioStrings.text("m4.coding.diffCardTitle", locale: locale))
                .fabushiFont(.subheadline)
                .fontWeight(.semibold)
                .foregroundStyle(theme.textPrimary)
        }
    }

    @ViewBuilder
    private func diffMetricPill(locale: FabushiLocale) -> some View {
        let stat = viewModel.latestDiffStat
        Text(
            FabushiAgentStudioStrings.format(
                "m4.coding.diffSummaryFormat",
                locale: locale,
                Int32(clamping: stat.filesChanged),
                Int32(clamping: stat.insertions),
                Int32(clamping: stat.deletions)
            )
        )
        .fabushiFont(.monospacedCaption)
        .foregroundStyle(theme.textSecondary)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(theme.surfaceSecondary, in: Capsule())
    }

    @ViewBuilder
    private func checkpointButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.coding.checkpointButton", locale: locale),
            systemImage: "clock.arrow.circlepath",
            accessibilityID: FabushiAccessibilityIDs.Stage.codingCheckpointButton,
            isProminent: false
        ) {
            isCheckpointsSheetPresented = true
        }
    }

    @ViewBuilder
    private func probeHostButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.coding.probeHost", locale: locale),
            systemImage: "antenna.radiowaves.left.and.right",
            accessibilityID: FabushiAccessibilityIDs.Inspector.codingHostProbeButton,
            isProminent: false
        ) {
            Task { @MainActor in
                _ = await viewModel.probeRemoteCodingHosts(locale: locale)
            }
        }
    }

    @ViewBuilder
    private func createPrButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.coding.createPR", locale: locale),
            systemImage: "arrow.triangle.pull",
            accessibilityID: FabushiAccessibilityIDs.Studio.codingAgentCreatePRButton,
            isProminent: true
        ) {
            isPullRequestSheetPresented = true
        }
    }

    @ViewBuilder
    private func codingApprovalDock(
        prompt: FabushiCodingLiveApprovalPrompt,
        locale: FabushiLocale,
        isCompact: Bool
    ) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "hand.raised.fill")
                    .foregroundStyle(theme.warningColor)
                    .accessibilityHidden(true)
                Text(prompt.title)
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
            }
            Text(prompt.detail)
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)

            HStack(spacing: 8) {
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.coding.allowOnce", locale: locale),
                    systemImage: "checkmark",
                    accessibilityID: FabushiAccessibilityIDs.Stage.toolApproveButton,
                    isProminent: true
                ) {
                    Task { @MainActor in
                        _ = await viewModel.resolveCodingApproval(
                            requestId: prompt.requestId,
                            decision: .allowOnce,
                            isChatStageSurface: true,
                            isCompact: isCompact
                        )
                    }
                }

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.coding.denyAction", locale: locale),
                    systemImage: "xmark",
                    accessibilityID: FabushiAccessibilityIDs.Stage.toolDenyButton,
                    isProminent: false
                ) {
                    Task { @MainActor in
                        _ = await viewModel.resolveCodingApproval(
                            requestId: prompt.requestId,
                            decision: .deny,
                            isChatStageSurface: true,
                            isCompact: isCompact
                        )
                    }
                }
            }
        }
        .padding(10)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.toolApprovalDock)
    }

    // MARK: - Modal Detail Sheets (Wrapped in `NavigationStack`)

    @ViewBuilder
    private func diffDetailSheet(locale: FabushiLocale, isCompact: Bool) -> some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text(FabushiAgentStudioStrings.text("m4.coding.remoteOnlyNotice", locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)

                    if let thread = viewModel.activeThread {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(thread.workspacePath)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textPrimary)
                            if let wt = thread.worktreePath, !wt.isEmpty {
                                Text("\(FabushiAgentStudioStrings.text("m4.coding.worktreeLabel", locale: locale)): \(wt)")
                                    .fabushiFont(.monospacedCaption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }

                    if !viewModel.latestDiffStat.changedFiles.isEmpty {
                        ForEach(viewModel.latestDiffStat.changedFiles, id: \.self) { file in
                            Text(file)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textPrimary)
                        }
                    }

                    if !viewModel.streamingDiffPreview.isEmpty {
                        Text(viewModel.streamingDiffPreview)
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textPrimary)
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }

                    FabushiGlassPillButton(
                        title: FabushiAgentStudioStrings.text("m4.common.refresh", locale: locale),
                        systemImage: "arrow.clockwise",
                        accessibilityID: "fabushi.internal.coding.refreshDiffButton",
                        isProminent: false
                    ) {
                        Task { @MainActor in
                            _ = await viewModel.dispatchRemoteCodingCommand(
                                "/diff",
                                isChatStageSurface: true,
                                isCompact: isCompact
                            )
                        }
                    }
                }
                .padding(16)
            }
            .background(theme.surfacePrimary)
            .navigationTitle(FabushiAgentStudioStrings.text("m4.coding.diffCardTitle", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.close", locale: locale)) {
                        isDiffSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }

    @ViewBuilder
    private func checkpointsSheet(locale: FabushiLocale, isCompact: Bool) -> some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let thread = viewModel.activeThread {
                        VStack(alignment: .leading, spacing: 8) {
                            TextField("Before Ref", text: $checkpointBeforeRef)
                                .fabushiFont(.monospacedCaption)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .padding(8)
                                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 8, style: .continuous))

                            TextField("After Ref", text: $checkpointAfterRef)
                                .fabushiFont(.monospacedCaption)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .padding(8)
                                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 8, style: .continuous))

                            FabushiGlassPillButton(
                                title: FabushiAgentStudioStrings.text("m4.coding.saveCheckpoint", locale: locale),
                                systemImage: "bookmark.fill",
                                accessibilityID: "fabushi.internal.coding.saveCheckpointButton",
                                isProminent: true
                            ) {
                                let nextTurn = (viewModel.checkpoints.map(\.turnIdx).max() ?? 0) + 1
                                Task { @MainActor in
                                    _ = await viewModel.saveCheckpoint(
                                        threadId: thread.threadId,
                                        turnIdx: nextTurn,
                                        beforeRef: checkpointBeforeRef,
                                        afterRef: checkpointAfterRef
                                    )
                                }
                            }
                        }
                        .padding(12)
                        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }

                    if viewModel.checkpoints.isEmpty {
                        Text(FabushiAgentStudioStrings.text("m4.coding.noCheckpoints", locale: locale))
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textSecondary)
                    } else {
                        ForEach(viewModel.checkpoints) { cp in
                            VStack(alignment: .leading, spacing: 6) {
                                HStack {
                                    Text("Turn #\(cp.turnIdx)")
                                        .fabushiFont(.headline)
                                        .foregroundStyle(theme.textPrimary)
                                    Spacer(minLength: 4)
                                    Text("\(cp.beforeRef) -> \(cp.afterRef)")
                                        .fabushiFont(.monospacedCaption)
                                        .foregroundStyle(theme.textSecondary)
                                }
                                if !cp.promptPreview.isEmpty {
                                    Text(cp.promptPreview)
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.textSecondary)
                                }
                                FabushiGlassPillButton(
                                    title: FabushiAgentStudioStrings.text("m4.coding.rollbackCheckpoint", locale: locale),
                                    systemImage: "arrow.uturn.backward",
                                    accessibilityID: "fabushi.internal.coding.rollback.\(cp.turnIdx)",
                                    isProminent: false
                                ) {
                                    Task { @MainActor in
                                        _ = await viewModel.dispatchRemoteCodingCommand(
                                            "/rollback \(cp.turnIdx)",
                                            isChatStageSurface: true,
                                            isCompact: isCompact
                                        )
                                    }
                                }
                            }
                            .padding(10)
                            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        }
                    }
                }
                .padding(16)
            }
            .background(theme.surfacePrimary)
            .navigationTitle(FabushiAgentStudioStrings.text("m4.coding.checkpointsSection", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.close", locale: locale)) {
                        isCheckpointsSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }

    @ViewBuilder
    private func pullRequestSheet(locale: FabushiLocale, isCompact: Bool) -> some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.coding.prTitlePlaceholder", locale: locale),
                        text: $prTitleDraft
                    )
                }
            }
            .navigationTitle(FabushiAgentStudioStrings.text("m4.coding.createPR", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                        isPullRequestSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(FabushiAgentStudioStrings.text("m4.coding.createPR", locale: locale)) {
                        let cmd = prTitleDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? "/pr"
                            : "/pr \(prTitleDraft)"
                        Task { @MainActor in
                            _ = await viewModel.dispatchRemoteCodingCommand(
                                cmd,
                                isChatStageSurface: true,
                                isCompact: isCompact
                            )
                            isPullRequestSheetPresented = false
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
