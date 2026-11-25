import Foundation
import SwiftUI

// MARK: - Reduce-Motion-Safe Grok Persona Avatar

@MainActor
public struct FabushiMultiAgentGrokPersonaAvatar: View {
    public let agentId: String
    public let hueName: String
    public let isSpeaking: Bool

    @ScaledMetric(relativeTo: .headline) private var avatarSize: CGFloat = 32
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var pulseActive = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(agentId: String, hueName: String = "purple", isSpeaking: Bool = false) {
        self.agentId = agentId
        self.hueName = hueName
        self.isSpeaking = isSpeaking
    }

    public var body: some View {
        let hue = FabushiPeerColorPalette.botHue(named: hueName, isDark: theme.preset.isDark)
        let reduceMotion = theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)
        let size = min(max(avatarSize, 28), 48)
        let monogram = String(agentId.trimmingCharacters(in: .whitespacesAndNewlines).prefix(2)).uppercased()

        ZStack {
            if isSpeaking && !reduceMotion {
                Circle()
                    .stroke(Color(fabushiHex: hue.pillTextRGB).opacity(pulseActive ? 0.15 : 0.55), lineWidth: 2)
                    .frame(width: size + 6, height: size + 6)
                    .scaleEffect(pulseActive ? 1.12 : 0.96)
                    .animation(
                        .easeInOut(duration: 1.1).repeatForever(autoreverses: true),
                        value: pulseActive
                    )
            } else if isSpeaking {
                Circle()
                    .stroke(Color(fabushiHex: hue.pillTextRGB), lineWidth: 2)
                    .frame(width: size + 4, height: size + 4)
            }

            Circle()
                .fill(Color(fabushiHex: hue.bubbleFillRGB))
                .frame(width: size, height: size)
                .overlay {
                    if monogram.isEmpty {
                        Image(systemName: "sparkles")
                            .fabushiFont(.caption)
                            .foregroundStyle(Color.white)
                    } else {
                        Text(monogram)
                            .fabushiFont(.caption2)
                            .fontWeight(.bold)
                            .foregroundStyle(Color.white)
                    }
                }
        }
        .accessibilityHidden(true)
        .onAppear {
            if isSpeaking && !reduceMotion {
                pulseActive = true
            }
        }
        .onChange(of: isSpeaking) { _, speaking in
            pulseActive = speaking && !reduceMotion
        }
    }
}

// MARK: - Inline Multi-Agent Room Accessories Banner (`makeMultiAgentRoomAccessories`)

/// Self-sizing inline stage banner (ZERO nested `NavigationStack`, ZERO full-height `List`)
/// mounting `Stage.floorControlBanner`, `Stage.floorInterruptButton`, 5-hop `LoopBudget` meter,
/// and `Inspector.multiAgentWorkspaceBadge` opening the Workspace & `D14-01` Diagnostics sheet.
@MainActor
public struct FabushiMultiAgentRoomAccessoriesView: View {
    public let chatId: Int64
    public let isChatStageSurface: Bool

    @State private var viewModel: FabushiMultiAgentViewModel
    @State private var isInspectorSheetPresented = false

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        chatId: Int64,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub(),
        isChatStageSurface: Bool = true
    ) {
        self.chatId = chatId
        self.isChatStageSurface = isChatStageSurface
        self._viewModel = State(
            initialValue: FabushiMultiAgentViewModel(
                chatId: chatId,
                coordinator: coordinator,
                router: router,
                cleanupHub: cleanupHub
            )
        )
    }

    public var body: some View {
        let locale = theme.locale
        let reduceMotion = theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)

        VStack(alignment: .leading, spacing: 8) {
            // Top Floor Control Banner (`fabushi.stage.floorControlBanner`)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    floorIdentityCluster(locale: locale)
                    Spacer(minLength: 8)
                    interruptButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    floorIdentityCluster(locale: locale)
                    interruptButton(locale: locale)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.floorControlBanner)

            // Bottom Loop Budget 5-Hop Meter + SharedGroupWorkspace Revision Badge
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    loopBudgetMeter(locale: locale)
                    Spacer(minLength: 8)
                    workspaceBadgeButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    loopBudgetMeter(locale: locale)
                    workspaceBadgeButton(locale: locale)
                }
            }
        }
        .padding(12)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
        )
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.2), value: viewModel.floorStatus.currentSpeaker)
        .task(id: "\(chatId):\(viewModel.coordinator.accountScopeGeneration)") {
            await viewModel.refreshRoomState()
        }
        .onChange(of: viewModel.coordinator.accountScopeGeneration) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .onChange(of: viewModel.coordinator.activeAccountId) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .sheet(isPresented: $isInspectorSheetPresented) {
            NavigationStack {
                FabushiMultiAgentInspectorSheetView(
                    viewModel: viewModel,
                    isChatStageSurface: isChatStageSurface,
                    onDismiss: { isInspectorSheetPresented = false }
                )
            }
            .environment(theme)
        }
    }

    // MARK: - Subviews

    @ViewBuilder
    private func floorIdentityCluster(locale: FabushiLocale) -> some View {
        let speaker = viewModel.floorStatus.currentSpeaker
        let grant = viewModel.floorStatus.activeGrant

        HStack(spacing: 10) {
            FabushiMultiAgentGrokPersonaAvatar(
                agentId: speaker ?? "AI",
                hueName: speaker == nil ? "cyan" : "purple",
                isSpeaking: speaker != nil
            )

            VStack(alignment: .leading, spacing: 3) {
                Text(
                    speaker.map {
                        FabushiAgentStudioStrings.format("m4.multiAgent.speakingFormat", locale: locale, $0)
                    } ?? FabushiAgentStudioStrings.text("m4.multiAgent.idleSpeaker", locale: locale)
                )
                .fabushiFont(.subheadline)
                .fontWeight(.semibold)
                .foregroundStyle(theme.textPrimary)
                .lineLimit(2)

                HStack(spacing: 8) {
                    if viewModel.floorStatus.queuedCount > 0 {
                        Text(
                            FabushiAgentStudioStrings.format(
                                "m4.multiAgent.queuedFormat",
                                locale: locale,
                                Int32(clamping: viewModel.floorStatus.queuedCount)
                            )
                        )
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                    }

                    if let grant {
                        TimelineView(.periodic(from: .now, by: 1.0)) { context in
                            let nowMs = Int64(context.date.timeIntervalSince1970 * 1000)
                            let remaining = grant.remainingLeaseSeconds(atMs: nowMs)
                            Text(
                                FabushiAgentStudioStrings.format(
                                    "m4.multiAgent.leaseRemainingFormat",
                                    locale: locale,
                                    Int32(clamping: remaining)
                                )
                            )
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(remaining <= 15 ? theme.warningColor : theme.textSecondary)
                        }

                        if grant.isWindingDown {
                            Text(FabushiAgentStudioStrings.text("m4.multiAgent.windingDownBadge", locale: locale))
                                .fabushiFont(.caption2)
                                .fontWeight(.semibold)
                                .foregroundStyle(theme.warningColor)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(theme.warningColor.opacity(0.15), in: Capsule())
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func interruptButton(locale: FabushiLocale) -> some View {
        let label = FabushiAgentStudioStrings.text("m4.multiAgent.interruptButton", locale: locale)
        Button {
            Task { @MainActor in
                let res = await viewModel.preemptFloorForUser()
                if res != nil {
                    let msg = FabushiAgentStudioStrings.text("m4.multiAgent.interruptedToast", locale: locale)
                    emitToast(msg, isError: false)
                }
            }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: "hand.raised.fill")
                    .accessibilityHidden(true)
                Text(label)
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .lineLimit(1)
            }
            .foregroundStyle(theme.destructiveColor)
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .fabushiMinimumTouchTarget()
            .background(theme.destructiveColor.opacity(0.14), in: Capsule())
            .overlay(Capsule().strokeBorder(theme.destructiveColor.opacity(0.4), lineWidth: 0.75))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.floorInterruptButton)
    }

    @ViewBuilder
    private func loopBudgetMeter(locale: FabushiLocale) -> some View {
        let current = Int(viewModel.loopBudget.currentHop)
        let maxHops = max(1, Int(viewModel.loopBudget.maxHops))
        let exhausted = viewModel.loopBudget.exhausted

        HStack(spacing: 8) {
            HStack(spacing: 3) {
                ForEach(0..<maxHops, id: \.self) { index in
                    Capsule()
                        .fill(
                            index < current
                                ? (exhausted ? theme.destructiveColor : theme.accentColor)
                                : theme.hairlineBorder
                        )
                        .frame(width: 14, height: 6)
                }
            }
            .accessibilityHidden(true)

            Text(
                exhausted
                    ? FabushiAgentStudioStrings.text("m4.multiAgent.loopExhaustedBadge", locale: locale)
                    : FabushiAgentStudioStrings.format(
                        "m4.multiAgent.loopBudgetFormat",
                        locale: locale,
                        Int32(clamping: current),
                        Int32(clamping: maxHops)
                    )
            )
            .fabushiFont(.caption)
            .foregroundStyle(exhausted ? theme.destructiveColor : theme.textSecondary)
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private func workspaceBadgeButton(locale: FabushiLocale) -> some View {
        let title = FabushiAgentStudioStrings.format(
            "m4.multiAgent.workspaceBadgeFormat",
            locale: locale,
            viewModel.workspace.revision
        )
        Button {
            isInspectorSheetPresented = true
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "doc.text.magnifyingglass")
                    .accessibilityHidden(true)
                Text(title)
                    .fabushiFont(.caption)
                    .fontWeight(.medium)
                    .lineLimit(1)
            }
            .foregroundStyle(theme.accentColor)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .fabushiMinimumTouchTarget()
            .background(theme.accentColor.opacity(0.12), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.multiAgentWorkspaceBadge)
    }

    private func emitToast(_ message: String, isError: Bool) {
        if isChatStageSurface {
            let shown = viewModel.router.showChatToastIfVisible(
                message,
                chatId: chatId,
                coordinator: viewModel.coordinator,
                isCompact: horizontalSizeClass == .compact,
                isError: isError
            )
            if !shown {
                viewModel.router.toastController.showStagePill(message, chatId: chatId)
            }
        } else {
            viewModel.router.toastController.showNonChatToast(message, isError: isError)
        }
    }
}

// MARK: - Multi-Agent Inspector & `D14-01` Wake Gate Diagnostics Sheet

@MainActor
private struct FabushiMultiAgentInspectorSheetView: View {
    @Bindable var viewModel: FabushiMultiAgentViewModel
    let isChatStageSurface: Bool
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var floorAgentIdInput = "rust_bot"
    @State private var workspaceKeyInput = "plan.md"
    @State private var workspaceValueInput = ""
    @State private var wakeMessageInput = "@rust_bot你好 请检查架构"
    @State private var wakeSenderIsHuman = true
    @State private var outboundDraftInput = "[PASS]"

    var body: some View {
        let locale = theme.locale

        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                floorControlSection(locale: locale)
                loopBudgetSection(locale: locale)
                sharedWorkspaceSection(locale: locale)
                wakeGateDiagnosticsSection(locale: locale)
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiAgentStudioStrings.text("m4.multiAgent.sheetTitle", locale: locale))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button(FabushiAgentStudioStrings.text("m4.common.close", locale: locale)) {
                    onDismiss()
                }
                .fabushiMinimumTouchTarget()
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { @MainActor in await viewModel.refreshRoomState() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .fabushiMinimumTouchTarget()
                .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.refresh", locale: locale))
            }
        }
    }

    // MARK: - 1. Floor Control Section

    @ViewBuilder
    private func floorControlSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.multiAgent.floorSection", locale: locale))

            TextField(
                FabushiAgentStudioStrings.text("m4.multiAgent.agentIdPlaceholder", locale: locale),
                text: $floorAgentIdInput
            )
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityIdentifier("fabushi.internal.multiAgent.floorAgentField")

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    requestFloorPill(locale: locale)
                    releaseFloorPill(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    requestFloorPill(locale: locale)
                    releaseFloorPill(locale: locale)
                }
            }
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    @ViewBuilder
    private func requestFloorPill(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.multiAgent.requestFloorButton", locale: locale),
            systemImage: "mic.fill",
            accessibilityID: "fabushi.internal.multiAgent.requestFloorButton",
            isProminent: true
        ) {
            Task { @MainActor in
                _ = await viewModel.requestFloor(
                    agentId: floorAgentIdInput,
                    priority: .directMention,
                    leaseMs: FabushiMultiAgentConstants.groupFloorLeaseMs
                )
            }
        }
    }

    @ViewBuilder
    private func releaseFloorPill(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.multiAgent.releaseFloorButton", locale: locale),
            systemImage: "mic.slash",
            accessibilityID: "fabushi.internal.multiAgent.releaseFloorButton",
            isProminent: false
        ) {
            let target = viewModel.floorStatus.currentSpeaker ?? floorAgentIdInput
            Task { @MainActor in
                _ = await viewModel.releaseFloor(agentId: target)
            }
        }
    }

    // MARK: - 2. 5-Hop Loop Budget Section

    @ViewBuilder
    private func loopBudgetSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.multiAgent.loopSection", locale: locale))

            Text(
                FabushiAgentStudioStrings.format(
                    "m4.multiAgent.loopBudgetFormat",
                    locale: locale,
                    Int32(clamping: viewModel.loopBudget.currentHop),
                    Int32(clamping: viewModel.loopBudget.maxHops)
                )
            )
            .fabushiFont(.body)
            .foregroundStyle(viewModel.loopBudget.exhausted ? theme.destructiveColor : theme.textPrimary)

            if !viewModel.loopBudget.trace.isEmpty {
                Text(viewModel.loopBudget.trace.joined(separator: " → "))
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textSecondary)
            }

            HStack(spacing: 8) {
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.multiAgent.recordHopButton", locale: locale),
                    systemImage: "arrow.triangle.branch",
                    accessibilityID: "fabushi.internal.multiAgent.recordHopButton",
                    isProminent: false
                ) {
                    Task { @MainActor in
                        _ = await viewModel.recordLoopHop(agentId: floorAgentIdInput)
                    }
                }

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.multiAgent.resetBudgetButton", locale: locale),
                    systemImage: "arrow.counterclockwise",
                    accessibilityID: "fabushi.internal.multiAgent.resetBudgetButton",
                    isProminent: false
                ) {
                    Task { @MainActor in
                        _ = await viewModel.resetLoopBudget()
                    }
                }
            }
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    // MARK: - 3. Revision-Fenced Shared Workspace Section

    @ViewBuilder
    private func sharedWorkspaceSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.multiAgent.workspaceSection", locale: locale))
                Text("rev \(viewModel.workspace.revision)")
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.accentColor)
            }

            if let conflict = viewModel.workspaceConflict {
                VStack(alignment: .leading, spacing: 8) {
                    Text(FabushiAgentStudioStrings.text("m4.multiAgent.workspaceConflictTitle", locale: locale))
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.warningColor)
                    Text(conflict.message)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                    HStack(spacing: 8) {
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.multiAgent.workspaceConflictReload", locale: locale),
                            accessibilityID: "fabushi.internal.multiAgent.conflictReloadButton",
                            isProminent: false
                        ) {
                            Task { @MainActor in await viewModel.dismissWorkspaceConflictAndReload() }
                        }
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.multiAgent.workspaceConflictOverwrite", locale: locale),
                            accessibilityID: "fabushi.internal.multiAgent.conflictOverwriteButton",
                            isProminent: true
                        ) {
                            Task { @MainActor in _ = await viewModel.overwriteWorkspaceConflict() }
                        }
                    }
                }
                .padding(10)
                .background(theme.warningColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }

            TextField(
                FabushiAgentStudioStrings.text("m4.multiAgent.workspaceKeyPlaceholder", locale: locale),
                text: $workspaceKeyInput
            )
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityIdentifier("fabushi.internal.multiAgent.workspaceKeyField")

            TextEditor(text: $workspaceValueInput)
                .frame(minHeight: 72)
                .padding(8)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("fabushi.internal.multiAgent.workspaceValueField")

            FabushiGlassPillButton(
                title: FabushiAgentStudioStrings.text("m4.multiAgent.workspaceSaveButton", locale: locale),
                systemImage: "checkmark.seal",
                accessibilityID: "fabushi.internal.multiAgent.workspaceSaveButton",
                isProminent: true
            ) {
                let currentRev = viewModel.workspace.revision
                let key = workspaceKeyInput
                let rawValue = workspaceValueInput
                Task { @MainActor in
                    _ = await viewModel.setWorkspaceEntry(
                        key: key,
                        value: .string(rawValue),
                        expectedRevision: currentRev
                    )
                }
            }

            ForEach(viewModel.workspace.sortedEntries) { entry in
                HStack(alignment: .top, spacing: 8) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(entry.key)
                            .fabushiFont(.subheadline)
                            .fontWeight(.semibold)
                            .foregroundStyle(theme.textPrimary)
                        Text(entry.displayValueText)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textSecondary)
                            .lineLimit(3)
                        Text(entry.updatedBy)
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                    }
                    Spacer()
                    Button(role: .destructive) {
                        Task { @MainActor in
                            _ = await viewModel.removeWorkspaceEntry(key: entry.key)
                        }
                    } label: {
                        Image(systemName: "trash")
                            .foregroundStyle(theme.destructiveColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.delete", locale: locale))
                }
                .padding(10)
                .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    // MARK: - 4. `D14-01` Wake Gate & Stay-Silent `[PASS]` Diagnostics Panel

    @ViewBuilder
    private func wakeGateDiagnosticsSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.multiAgent.wakeGateSection", locale: locale))

            TextField(
                FabushiAgentStudioStrings.text("m4.multiAgent.wakeGateMessagePlaceholder", locale: locale),
                text: $wakeMessageInput
            )
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityIdentifier("fabushi.internal.multiAgent.wakeMessageField")

            FabushiGlassPillButton(
                title: FabushiAgentStudioStrings.text("m4.multiAgent.wakeGateRunButton", locale: locale),
                systemImage: "waveform.path.ecg",
                accessibilityID: "fabushi.internal.multiAgent.wakeGateRunButton",
                isProminent: true
            ) {
                let candidates = [
                    FabushiMultiAgentWakeBotCandidateDTO(
                        botUserId: FabushiMultiAgentConstants.officialDefaultBotUserId,
                        agentId: "fabushi_default",
                        name: "Fabushi Assistant",
                        username: "fabushi_bot",
                        role: "generalist",
                        skills: ["general", "planning"]
                    ),
                    FabushiMultiAgentWakeBotCandidateDTO(
                        botUserId: 900_002,
                        agentId: "rust_bot",
                        name: "Rust Architect",
                        username: "rust_bot",
                        role: "systems engineer",
                        skills: ["rust", "cargo", "ffi"]
                    ),
                ]
                let input = FabushiMultiAgentWakeGateInputDTO(
                    candidates: candidates,
                    messageText: wakeMessageInput,
                    senderIsHuman: wakeSenderIsHuman,
                    remainingLoopBudget: viewModel.loopBudget.remainingHops
                )
                Task { @MainActor in
                    _ = await viewModel.evaluateWakeGate(input)
                }
            }

            if let decision = viewModel.lastWakeDecision {
                VStack(alignment: .leading, spacing: 4) {
                    Text(FabushiAgentStudioStrings.text(decision.reason.localizationKey, locale: locale))
                        .fabushiFont(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(decision.shouldWake ? theme.successColor : theme.textSecondary)
                    Text("outcome: \(decision.outcome.rawValue) · wake_ids: \(decision.wakeBotUserIds.map(String.init).joined(separator: ", "))")
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(theme.textSecondary)
                }
                .padding(10)
                .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }

            Divider().overlay(theme.separatorHairline)

            TextField(
                FabushiAgentStudioStrings.text("m4.multiAgent.outboundDraftPlaceholder", locale: locale),
                text: $outboundDraftInput
            )
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityIdentifier("fabushi.internal.multiAgent.outboundDraftField")

            FabushiGlassPillButton(
                title: FabushiAgentStudioStrings.text("m4.multiAgent.outboundReviewButton", locale: locale),
                systemImage: "checkmark.bubble",
                accessibilityID: "fabushi.internal.multiAgent.outboundReviewButton",
                isProminent: false
            ) {
                let draft = outboundDraftInput
                Task { @MainActor in
                    _ = await viewModel.checkIsPassSentinel(draft)
                    _ = await viewModel.evaluateOutboundReview(
                        FabushiMultiAgentOutboundReviewInputDTO(
                            agentId: floorAgentIdInput,
                            candidateText: draft
                        )
                    )
                }
            }

            if let isPass = viewModel.lastPassSentinelResult {
                Text(
                    FabushiAgentStudioStrings.text(
                        isPass ? "m4.multiAgent.passSentinelYes" : "m4.multiAgent.passSentinelNo",
                        locale: locale
                    )
                )
                .fabushiFont(.footnote)
                .foregroundStyle(isPass ? theme.warningColor : theme.successColor)
            }
            if let verdict = viewModel.lastOutboundVerdict {
                Text("review: \(verdict.outcome.rawValue) (\(verdict.reason.rawValue))")
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textSecondary)
            }
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }
}
