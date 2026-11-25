import Foundation
import SwiftUI

// MARK: - 3-Tier Classified AI Provider Error Banner (`D10-10`)

@MainActor
internal struct FabushiStudioClassifiedErrorBanner: View {
    let error: FabushiMagpieClassifiedError
    let onOpenAiProviders: () -> Void
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        let locale = theme.locale
        if error.showSettingsCTA {
            FabushiProviderErrorCard(
                detail: error.detail(locale: locale),
                onOpenAIProviders: onOpenAiProviders,
                onDismiss: onDismiss
            )
        } else {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.octagon.fill")
                        .foregroundStyle(theme.warningColor)
                        .accessibilityHidden(true)
                    Text(error.heading(locale: locale))
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(2)
                    Spacer(minLength: 4)
                    Button(action: onDismiss) {
                        Image(systemName: "xmark")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(FabushiAgentStudioStrings.text("m4.error.dismissHint", locale: locale))
                }

                Text(error.detail(locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(14)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
            .overlay(
                RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                    .strokeBorder(theme.warningColor.opacity(0.6), lineWidth: 1.0)
            )
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("fabushi.internal.studio.classifiedErrorBanner")
        }
    }
}

// MARK: - Tab 1: Overview, 29 Toolsets, 16 Browser Tools & Connected Consoles

@MainActor
internal struct FabushiStudioOverviewTabView: View {
    @Bindable var viewModel: FabushiAgentStudioViewModel
    let isRegularWidth: Bool
    let onOpenMcpConsole: () -> Void
    let onOpenMultiAgentLab: () -> Void
    let codingAgentsPanelContent: AnyView

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var showBrowserToolsCatalog = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 16) {
            runtimeMetricsSection(locale: locale)
            connectedConsolesSection(locale: locale)
            codingAgentsPanelContent
            toolsetsSection(locale: locale)
            if !viewModel.bots.isEmpty {
                botsSection(locale: locale)
            }
        }
    }

    @ViewBuilder
    private func runtimeMetricsSection(locale: FabushiLocale) -> some View {
        let columns = isRegularWidth
            ? [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)]
            : [GridItem(.flexible())]

        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.overview.runtimeSection", locale: locale))

            LazyVGrid(columns: columns, spacing: 10) {
                metricCard(
                    title: FabushiAgentStudioStrings.text("m4.studio.overview.engineLabel", locale: locale),
                    value: viewModel.remoteStatus?.activeModel
                        ?? viewModel.localStatus?.engine
                        ?? "hermes_embedded",
                    subtitle: viewModel.remoteStatus != nil
                        ? FabushiAgentStudioStrings.text("m4.common.remoteMode", locale: locale)
                        : FabushiAgentStudioStrings.text("m4.common.localMode", locale: locale),
                    systemImage: "cpu"
                )
                metricCard(
                    title: FabushiAgentStudioStrings.text("m4.studio.overview.toolsetsLabel", locale: locale),
                    value: "\(viewModel.toolsetSummary?.enabledCount ?? 0) / \(viewModel.toolsetSummary?.totalToolsets ?? 29)",
                    subtitle: FabushiAgentStudioStrings.format(
                        "m4.studio.toolsets.toolCountFormat",
                        locale: locale,
                        Int32(clamping: viewModel.toolsetSummary?.activeToolCount ?? 0)
                    ),
                    systemImage: "wrench.and.screwdriver"
                )
                metricCard(
                    title: FabushiAgentStudioStrings.text("m4.studio.overview.commandsLabel", locale: locale),
                    value: "\(viewModel.localStatus?.commandsCount ?? viewModel.commands.count)",
                    subtitle: "/status · /skills · /distill",
                    systemImage: "command"
                )
                metricCard(
                    title: FabushiAgentStudioStrings.text("m4.studio.overview.storageLabel", locale: locale),
                    value: (viewModel.localStatus?.journalMode ?? "wal").uppercased(),
                    subtitle: (viewModel.localStatus?.fts5Available ?? true)
                        ? FabushiAgentStudioStrings.text("m4.studio.overview.fts5Available", locale: locale)
                        : FabushiAgentStudioStrings.text("m4.studio.overview.fts5Fallback", locale: locale),
                    systemImage: "internaldrive"
                )
            }
        }
    }

    @ViewBuilder
    private func metricCard(
        title: String,
        value: String,
        subtitle: String,
        systemImage: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)
                Text(title)
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
            Text(value)
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
                .lineLimit(2)
            Text(subtitle)
                .fabushiFont(.caption2)
                .foregroundStyle(theme.textMuted)
                .lineLimit(2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
        )
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private func connectedConsolesSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.overview.quickActions", locale: locale))

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    mcpGrantsButton(locale: locale)
                    openMagpieButton(locale: locale)
                    multiAgentLabButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    mcpGrantsButton(locale: locale)
                    openMagpieButton(locale: locale)
                    multiAgentLabButton(locale: locale)
                }
            }
        }
    }

    @ViewBuilder
    private func mcpGrantsButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.studio.overview.openMcp", locale: locale),
            systemImage: "key.viewfinder",
            accessibilityID: FabushiAccessibilityIDs.Studio.mcpGrantsButton,
            isProminent: true,
            action: onOpenMcpConsole
        )
    }

    @ViewBuilder
    private func openMagpieButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.studio.overview.openMagpie", locale: locale),
            systemImage: "slider.horizontal.3",
            accessibilityID: "fabushi.internal.studio.openMagpieButton",
            isProminent: false
        ) {
            viewModel.router.navigate(to: .settings(section: "aiProviders"), coordinator: viewModel.coordinator)
        }
    }

    @ViewBuilder
    private func multiAgentLabButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.studio.overview.openMultiAgent", locale: locale),
            systemImage: "person.3.sequence.fill",
            accessibilityID: "fabushi.internal.studio.openMultiAgentButton",
            isProminent: false,
            action: onOpenMultiAgentLab
        )
    }

    @ViewBuilder
    private func toolsetsSection(locale: FabushiLocale) -> some View {
        let items: [FabushiStudioToolsetItemDTO] = viewModel.toolsetSummary?.toolsets
            ?? FabushiStudioToolsetCatalog.allToolsets.map {
                FabushiStudioToolsetItemDTO(id: $0.id, enabled: $0.isRequiredCore, description: "", tools: [])
            }

        VStack(alignment: .leading, spacing: 12) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.toolsets.sectionTitle", locale: locale))

            if let sandboxNotice = viewModel.sandboxNoticeMessage {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "lock.shield.fill")
                        .foregroundStyle(theme.warningColor)
                        .accessibilityHidden(true)
                    Text(sandboxNotice)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textPrimary)
                }
                .padding(10)
                .background(theme.warningColor.opacity(0.14), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("fabushi.internal.studio.sandboxNoticeBanner")
            }

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(FabushiStudioToolsetCatalog.presets) { preset in
                        let isActive = viewModel.toolsetSummary?.activePreset == preset.id
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text(preset.titleKey, locale: locale),
                            systemImage: preset.iosAllowed ? "checkmark.circle" : "desktopcomputer",
                            accessibilityID: "fabushi.internal.studio.preset.\(preset.id)",
                            isProminent: isActive
                        ) {
                            Task { @MainActor in
                                _ = await viewModel.applyToolsetPreset(preset.id, locale: locale)
                            }
                        }
                    }
                }
            }

            VStack(spacing: 8) {
                ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                    toolsetRow(item: item, index: index, locale: locale)
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.toolsetList)

            // Expandable 16-Tool Browser Automation Catalog (`D11-03` highlighting 6 v1.6.11 tools)
            DisclosureGroup(
                isExpanded: $showBrowserToolsCatalog,
                content: {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(FabushiAgentStudioStrings.text("m4.studio.toolsets.browserIosNotice", locale: locale))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .padding(.top, 4)

                        ForEach(FabushiStudioToolsetCatalog.browserTools) { tool in
                            VStack(alignment: .leading, spacing: 3) {
                                HStack(spacing: 6) {
                                    Text(tool.name)
                                        .fabushiFont(.monospacedCaption)
                                        .fontWeight(.semibold)
                                        .foregroundStyle(theme.textPrimary)
                                    if tool.isV1611New {
                                        Text(FabushiAgentStudioStrings.text("m4.studio.toolsets.browserV1611Badge", locale: locale))
                                            .fabushiFont(.caption2)
                                            .fontWeight(.semibold)
                                            .foregroundStyle(theme.accentColor)
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(theme.accentColor.opacity(0.14), in: Capsule())
                                    }
                                }
                                Text(tool.summary(locale: locale))
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(8)
                            .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        }
                    }
                },
                label: {
                    Label(
                        FabushiAgentStudioStrings.text("m4.studio.toolsets.browserCatalogTitle", locale: locale),
                        systemImage: "safari"
                    )
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                }
            )
            .padding(12)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }

    @ViewBuilder
    private func toolsetRow(
        item: FabushiStudioToolsetItemDTO,
        index: Int,
        locale: FabushiLocale
    ) -> some View {
        let toggleBinding = Binding<Bool>(
            get: { item.enabled },
            set: { newValue in
                Task { @MainActor in
                    _ = await viewModel.setToolsetEnabled(item.id, enabled: newValue, locale: locale)
                }
            }
        )

        HStack(alignment: .center, spacing: 10) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(item.id)
                        .fabushiFont(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.textPrimary)
                    if item.isRequiredCore {
                        badgePill(
                            text: FabushiAgentStudioStrings.text("m4.studio.toolsets.coreLockedBadge", locale: locale),
                            color: theme.accentColor
                        )
                    } else if item.isIosSandboxGated {
                        badgePill(
                            text: FabushiAgentStudioStrings.text("m4.studio.toolsets.sandboxGatedBadge", locale: locale),
                            color: theme.warningColor
                        )
                    }
                }
                if !item.description.isEmpty {
                    Text(item.description)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                        .lineLimit(2)
                }
                Text(
                    FabushiAgentStudioStrings.format(
                        "m4.studio.toolsets.toolCountFormat",
                        locale: locale,
                        Int32(clamping: item.tools.count)
                    )
                )
                .fabushiFont(.caption2)
                .foregroundStyle(theme.textMuted)
            }

            Spacer(minLength: 8)

            Toggle(item.id, isOn: toggleBinding)
                .labelsHidden()
                .disabled(item.isRequiredCore)
                .accessibilityLabel(item.id)
                .accessibilityIdentifier(
                    index == 0
                        ? FabushiAccessibilityIDs.Studio.toolsetToggleFirst
                        : "fabushi.internal.studio.toolset.toggle.\(item.id)"
                )
        }
        .padding(12)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    @ViewBuilder
    private func botsSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.overview.botsSection", locale: locale))
            ForEach(viewModel.bots) { bot in
                HStack(spacing: 10) {
                    FabushiAvatarBadge(peerId: bot.botUserId, title: bot.displayName, isBot: true)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(bot.displayName)
                            .fabushiFont(.subheadline)
                            .fontWeight(.semibold)
                            .foregroundStyle(theme.textPrimary)
                        Text("@\(bot.username) · \(bot.preferredModel.isEmpty ? "default" : bot.preferredModel)")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    Spacer()
                    Text(bot.executionState)
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.accentColor)
                }
                .padding(10)
                .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            }
        }
    }

    @ViewBuilder
    private func badgePill(text: String, color: Color) -> some View {
        Text(text)
            .fabushiFont(.caption2)
            .fontWeight(.medium)
            .foregroundStyle(color)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(color.opacity(0.14), in: Capsule())
    }
}

// MARK: - Tab 2: Kanban Boards, Cards & Tasks FSM

@MainActor
internal struct FabushiStudioKanbanTabView: View {
    @Bindable var viewModel: FabushiAgentStudioViewModel
    let isRegularWidth: Bool

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var newTaskIdInput = ""
    @State private var showCreateBoardSheet = false
    @State private var showCreateCardSheet = false
    @State private var newBoardName = ""
    @State private var newBoardDesc = ""
    @State private var newCardTitle = ""
    @State private var newCardDesc = ""
    @State private var newCardColumn = "Backlog"
    @State private var newCardPriority = "medium"
    @State private var compactSelectedColumnIndex = 0

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 18) {
            tasksFsmSection(locale: locale)
            kanbanBoardsSection(locale: locale)
        }
        .sheet(isPresented: $showCreateBoardSheet) {
            NavigationStack {
                Form {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.studio.kanban.boardNamePlaceholder", locale: locale),
                        text: $newBoardName
                    )
                    TextField(
                        FabushiAgentStudioStrings.text("m4.studio.kanban.boardDescPlaceholder", locale: locale),
                        text: $newBoardDesc
                    )
                }
                .navigationTitle(FabushiAgentStudioStrings.text("m4.studio.kanban.newBoard", locale: locale))
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                            showCreateBoardSheet = false
                        }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(FabushiAgentStudioStrings.text("m4.common.create", locale: locale)) {
                            let name = newBoardName
                            let desc = newBoardDesc
                            Task { @MainActor in
                                _ = await viewModel.createKanbanBoard(name: name, description: desc)
                                newBoardName = ""
                                newBoardDesc = ""
                                showCreateBoardSheet = false
                            }
                        }
                        .disabled(newBoardName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .environment(theme)
        }
        .sheet(isPresented: $showCreateCardSheet) {
            NavigationStack {
                Form {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.studio.kanban.cardTitlePlaceholder", locale: locale),
                        text: $newCardTitle
                    )
                    TextField(
                        FabushiAgentStudioStrings.text("m4.studio.kanban.cardDescPlaceholder", locale: locale),
                        text: $newCardDesc
                    )
                    if let activeBoard {
                        Picker(
                            FabushiAgentStudioStrings.text("m4.studio.kanban.moveCard", locale: locale),
                            selection: $newCardColumn
                        ) {
                            ForEach(activeBoard.columns, id: \.self) { col in
                                Text(col).tag(col)
                            }
                        }
                    }
                    Picker(
                        FabushiAgentStudioStrings.text("m4.studio.kanban.priorityLabel", locale: locale),
                        selection: $newCardPriority
                    ) {
                        Text("low").tag("low")
                        Text("medium").tag("medium")
                        Text("high").tag("high")
                    }
                }
                .navigationTitle(FabushiAgentStudioStrings.text("m4.studio.kanban.newCard", locale: locale))
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                            showCreateCardSheet = false
                        }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(FabushiAgentStudioStrings.text("m4.common.create", locale: locale)) {
                            guard let boardId = viewModel.selectedKanbanBoardId else { return }
                            let title = newCardTitle
                            let desc = newCardDesc
                            let col = newCardColumn
                            let prio = newCardPriority
                            Task { @MainActor in
                                _ = await viewModel.createKanbanCard(
                                    boardId: boardId,
                                    columnName: col,
                                    title: title,
                                    description: desc,
                                    priority: prio
                                )
                                newCardTitle = ""
                                newCardDesc = ""
                                showCreateCardSheet = false
                            }
                        }
                        .disabled(newCardTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .environment(theme)
        }
    }

    private var activeBoard: FabushiStudioKanbanBoardDTO? {
        viewModel.kanbanBoards.first { $0.id == viewModel.selectedKanbanBoardId } ?? viewModel.kanbanBoards.first
    }

    @ViewBuilder
    private func tasksFsmSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.kanban.tasksSection", locale: locale))

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    taskIdField(locale: locale)
                    createTaskButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    taskIdField(locale: locale)
                    createTaskButton(locale: locale)
                }
            }

            if viewModel.tasks.isEmpty {
                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.studio.kanban.noTasksTitle", locale: locale),
                    systemImage: "checklist",
                    descriptionText: FabushiAgentStudioStrings.text("m4.studio.kanban.noTasksDesc", locale: locale),
                    accessibilityID: "fabushi.internal.studio.emptyTasks"
                )
            } else {
                ForEach(viewModel.tasks) { task in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text(task.taskId)
                                .fabushiFont(.subheadline)
                                .fontWeight(.semibold)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            Text(FabushiAgentStudioStrings.text(task.state.localizationKey, locale: locale))
                                .fabushiFont(.caption)
                                .fontWeight(.semibold)
                                .foregroundStyle(color(for: task.state))
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(color(for: task.state).opacity(0.14), in: Capsule())
                        }
                        if let abort = task.abortReason, !abort.isEmpty {
                            Text(abort)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.destructiveColor)
                        }
                        if !task.state.allowedNextStates.isEmpty {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 6) {
                                    ForEach(task.state.allowedNextStates, id: \.self) { nextState in
                                        Button {
                                            Task { @MainActor in
                                                _ = await viewModel.transitionTask(taskId: task.taskId, to: nextState)
                                            }
                                        } label: {
                                            Text(FabushiAgentStudioStrings.text(nextState.localizationKey, locale: locale))
                                                .fabushiFont(.caption)
                                                .foregroundStyle(theme.accentColor)
                                                .padding(.horizontal, 10)
                                                .padding(.vertical, 6)
                                                .fabushiMinimumTouchTarget()
                                                .background(theme.accentColor.opacity(0.12), in: Capsule())
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                            }
                        }
                    }
                    .padding(12)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                }
            }
        }
    }

    @ViewBuilder
    private func taskIdField(locale: FabushiLocale) -> some View {
        TextField(
            FabushiAgentStudioStrings.text("m4.studio.kanban.taskIdPlaceholder", locale: locale),
            text: $newTaskIdInput
        )
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .padding(10)
        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .accessibilityIdentifier("fabushi.internal.studio.taskIdField")
    }

    @ViewBuilder
    private func createTaskButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.studio.kanban.newTask", locale: locale),
            systemImage: "plus",
            accessibilityID: "fabushi.internal.studio.taskCreateButton",
            isProminent: true
        ) {
            let customId = newTaskIdInput
            Task { @MainActor in
                _ = await viewModel.createTask(taskId: customId.isEmpty ? nil : customId)
                newTaskIdInput = ""
            }
        }
    }

    @ViewBuilder
    private func kanbanBoardsSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            ViewThatFits(in: .horizontal) {
                HStack {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.kanban.boardsSection", locale: locale))
                    Spacer()
                    kanbanActionButtons(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.kanban.boardsSection", locale: locale))
                    kanbanActionButtons(locale: locale)
                }
            }

            if viewModel.kanbanBoards.isEmpty {
                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.studio.kanban.noBoardsTitle", locale: locale),
                    systemImage: "rectangle.split.3x1",
                    descriptionText: FabushiAgentStudioStrings.text("m4.studio.kanban.noBoardsDesc", locale: locale),
                    accessibilityID: "fabushi.internal.studio.emptyBoards"
                )
            } else if let board = activeBoard {
                if viewModel.kanbanBoards.count > 1 {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(viewModel.kanbanBoards) { b in
                                FabushiGlassPillButton(
                                    title: b.name,
                                    accessibilityID: "fabushi.internal.studio.board.\(b.id)",
                                    isProminent: b.id == board.id
                                ) {
                                    viewModel.selectedKanbanBoardId = b.id
                                    Task { @MainActor in await viewModel.loadKanbanCards(boardId: b.id) }
                                }
                            }
                        }
                    }
                }

                if isRegularWidth {
                    ScrollView(.horizontal, showsIndicators: true) {
                        HStack(alignment: .top, spacing: 12) {
                            ForEach(board.columns, id: \.self) { column in
                                kanbanColumnView(board: board, column: column, locale: locale)
                                    .frame(width: 260)
                            }
                        }
                    }
                } else {
                    let cols = board.columns.isEmpty ? ["Backlog", "In Progress", "Review", "Done"] : board.columns
                    let safeIdx = min(compactSelectedColumnIndex, max(0, cols.count - 1))
                    Picker("Column", selection: $compactSelectedColumnIndex) {
                        ForEach(Array(cols.enumerated()), id: \.offset) { idx, col in
                            Text(col).tag(idx)
                        }
                    }
                    .pickerStyle(.segmented)

                    kanbanColumnView(board: board, column: cols[safeIdx], locale: locale)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.kanbanBoard)
    }

    @ViewBuilder
    private func kanbanActionButtons(locale: FabushiLocale) -> some View {
        HStack(spacing: 8) {
            FabushiGlassPillButton(
                title: FabushiAgentStudioStrings.text("m4.studio.kanban.newBoard", locale: locale),
                systemImage: "rectangle.badge.plus",
                accessibilityID: "fabushi.internal.studio.newBoardButton",
                isProminent: false
            ) {
                showCreateBoardSheet = true
            }
            if let board = activeBoard {
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.studio.kanban.newCard", locale: locale),
                    systemImage: "plus.square",
                    accessibilityID: "fabushi.internal.studio.newCardButton",
                    isProminent: true
                ) {
                    newCardColumn = board.columns.first ?? "Backlog"
                    showCreateCardSheet = true
                }
            }
        }
    }

    @ViewBuilder
    private func kanbanColumnView(
        board: FabushiStudioKanbanBoardDTO,
        column: String,
        locale: FabushiLocale
    ) -> some View {
        let cards = viewModel.kanbanCards.filter { $0.columnName == column }
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(column)
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                Text("\(cards.count)")
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textMuted)
            }

            if cards.isEmpty {
                Text(FabushiAgentStudioStrings.text("m4.studio.kanban.emptyColumn", locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textMuted)
                    .padding(.vertical, 8)
            } else {
                ForEach(cards) { card in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(card.title)
                                .fabushiFont(.subheadline)
                                .fontWeight(.medium)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            Menu {
                                ForEach(board.columns.filter { $0 != card.columnName }, id: \.self) { target in
                                    Button(target) {
                                        Task { @MainActor in
                                            _ = await viewModel.moveKanbanCard(cardId: card.id, targetColumn: target)
                                        }
                                    }
                                }
                                Divider()
                                Button(
                                    FabushiAgentStudioStrings.text("m4.common.delete", locale: locale),
                                    role: .destructive
                                ) {
                                    Task { @MainActor in
                                        _ = await viewModel.deleteKanbanCard(cardId: card.id)
                                    }
                                }
                            } label: {
                                Image(systemName: "ellipsis.circle")
                                    .foregroundStyle(theme.textSecondary)
                                    .fabushiMinimumTouchTarget()
                            }
                            .accessibilityLabel(FabushiAgentStudioStrings.text("m4.studio.kanban.moveCard", locale: locale))
                        }

                        if !card.description.isEmpty {
                            Text(card.description)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(3)
                        }
                    }
                    .padding(10)
                    .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
        }
        .padding(12)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    private func color(for state: FabushiStudioTaskState) -> Color {
        switch state {
        case .pending: return theme.textSecondary
        case .running: return theme.accentColor
        case .waitingAuth, .waitingConnection: return theme.warningColor
        case .completed: return theme.successColor
        case .failed, .cancelled: return theme.destructiveColor
        }
    }
}

// MARK: - Tab 3: Skills (`SKILL.md` + `D11-04` Distilled `script_app`)

@MainActor
internal struct FabushiStudioSkillsTabView: View {
    @Bindable var viewModel: FabushiAgentStudioViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var showCreateSkillSheet = false
    @State private var newSkillName = ""
    @State private var newSkillDesc = ""
    @State private var newSkillBody = ""
    @State private var inspectedSkill: FabushiStudioSkillDocumentDTO?
    @State private var preflightSheetApp: FabushiStudioAutomationScriptApp?

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 14) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    filterField(locale: locale)
                    newSkillButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    filterField(locale: locale)
                    newSkillButton(locale: locale)
                }
            }

            if let replayNotice = viewModel.lastReplayNotice {
                Text(replayNotice)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textPrimary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }

            if viewModel.skills.isEmpty {
                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.studio.skills.emptyTitle", locale: locale),
                    systemImage: "wand.and.stars",
                    descriptionText: FabushiAgentStudioStrings.text("m4.studio.skills.emptyDesc", locale: locale),
                    accessibilityID: "fabushi.internal.studio.emptySkills"
                )
            } else {
                ForEach(viewModel.skills) { skill in
                    skillCard(skill, locale: locale)
                }
            }
        }
        .sheet(isPresented: $showCreateSkillSheet) {
            NavigationStack {
                Form {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.studio.skills.namePlaceholder", locale: locale),
                        text: $newSkillName
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    TextField(
                        FabushiAgentStudioStrings.text("m4.studio.skills.descPlaceholder", locale: locale),
                        text: $newSkillDesc
                    )
                    TextEditor(text: $newSkillBody)
                        .frame(minHeight: 140)
                }
                .navigationTitle(FabushiAgentStudioStrings.text("m4.studio.skills.newSkill", locale: locale))
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                            showCreateSkillSheet = false
                        }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button(FabushiAgentStudioStrings.text("m4.common.save", locale: locale)) {
                            let name = newSkillName
                            let desc = newSkillDesc
                            let body = newSkillBody
                            Task { @MainActor in
                                _ = await viewModel.createSkill(name: name, description: desc, body: body)
                                newSkillName = ""
                                newSkillDesc = ""
                                newSkillBody = ""
                                showCreateSkillSheet = false
                            }
                        }
                        .disabled(newSkillName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .environment(theme)
        }
        .sheet(item: $inspectedSkill) { skill in
            NavigationStack {
                skillInspectorSheet(skill, locale: locale)
            }
            .environment(theme)
        }
        .sheet(item: $preflightSheetApp) { app in
            NavigationStack {
                publishPreflightSheet(app: app, locale: locale)
            }
            .environment(theme)
        }
    }

    @ViewBuilder
    private func filterField(locale: FabushiLocale) -> some View {
        TextField(
            FabushiAgentStudioStrings.text("m4.studio.skills.searchPlaceholder", locale: locale),
            text: $viewModel.skillFilterQuery
        )
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .padding(10)
        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .onSubmit {
            Task { @MainActor in
                await viewModel.loadSkills(filter: viewModel.skillFilterQuery)
            }
        }
    }

    @ViewBuilder
    private func newSkillButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.studio.skills.newSkill", locale: locale),
            systemImage: "plus",
            accessibilityID: "fabushi.internal.studio.newSkillButton",
            isProminent: true
        ) {
            showCreateSkillSheet = true
        }
    }

    @ViewBuilder
    private func skillCard(_ skill: FabushiStudioSkillDocumentDTO, locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Text(skill.frontmatter.name)
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                if skill.isDistilledScriptApp {
                    Text(FabushiAgentStudioStrings.text("m4.studio.skills.distilledBadge", locale: locale))
                        .fabushiFont(.caption2)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.accentColor)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(theme.accentColor.opacity(0.14), in: Capsule())
                }
            }

            Text(skill.frontmatter.description)
                .fabushiFont(.footnote)
                .foregroundStyle(theme.textSecondary)

            HStack(spacing: 8) {
                Text("v\(skill.frontmatter.version) · \(skill.frontmatter.origin.displayName)")
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.textMuted)
                if skill.trusted {
                    Text(FabushiAgentStudioStrings.text("m4.studio.skills.trustedBadge", locale: locale))
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.successColor)
                }
            }

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    FabushiGlassPillButton(
                        title: FabushiAgentStudioStrings.text(
                            skill.isDistilledScriptApp ? "m4.studio.skills.inspectSteps" : "m4.common.inspect",
                            locale: locale
                        ),
                        systemImage: "list.bullet.rectangle",
                        accessibilityID: "fabushi.internal.studio.skill.inspect.\(skill.id)",
                        isProminent: false
                    ) {
                        inspectedSkill = skill
                    }

                    if skill.isDistilledScriptApp {
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.studio.skills.runDeterministic", locale: locale),
                            systemImage: "play.fill",
                            accessibilityID: "fabushi.internal.studio.skill.run.\(skill.id)",
                            isProminent: true
                        ) {
                            Task { @MainActor in
                                _ = await viewModel.runDistilledScriptApp(skill, locale: locale)
                            }
                        }

                        if let app = skill.extractAutomationScriptApp() {
                            FabushiGlassPillButton(
                                title: FabushiAgentStudioStrings.text("m4.studio.skills.publishPreflight", locale: locale),
                                systemImage: "checkmark.shield",
                                accessibilityID: "fabushi.internal.studio.skill.publish.\(skill.id)",
                                isProminent: false
                            ) {
                                _ = viewModel.validateScriptAppForPublish(app)
                                preflightSheetApp = app
                            }
                        }
                    }

                    if skill.frontmatter.origin != .bundled {
                        Button(role: .destructive) {
                            Task { @MainActor in
                                _ = await viewModel.deleteSkill(name: skill.frontmatter.name)
                            }
                        } label: {
                            Image(systemName: "trash")
                                .foregroundStyle(theme.destructiveColor)
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.delete", locale: locale))
                    }
                }
            }
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    @ViewBuilder
    private func skillInspectorSheet(_ skill: FabushiStudioSkillDocumentDTO, locale: FabushiLocale) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if let app = skill.extractAutomationScriptApp() {
                    HStack(spacing: 8) {
                        Image(systemName: "bolt.shield.fill")
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)
                        Text(
                            FabushiAgentStudioStrings.format(
                                "m4.studio.skills.stepsCountFormat",
                                locale: locale,
                                Int32(clamping: app.steps.count)
                            )
                        )
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                    }

                    if !app.parameters.isEmpty {
                        FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.skills.paramsSection", locale: locale))
                        ForEach(app.parameters) { param in
                            HStack {
                                Text(param.name)
                                    .fabushiFont(.monospacedCaption)
                                    .foregroundStyle(theme.textPrimary)
                                Spacer()
                                if param.isSecret {
                                    Text(FabushiAgentStudioStrings.text("m4.studio.skills.secretParamBadge", locale: locale))
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.warningColor)
                                }
                            }
                            .padding(8)
                            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        }
                    }

                    ForEach(Array(app.steps.enumerated()), id: \.offset) { idx, step in
                        HStack(alignment: .top, spacing: 8) {
                            Text("\(idx + 1).")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textMuted)
                            Text(step.displaySummary)
                                .fabushiFont(.footnote)
                                .foregroundStyle(theme.textPrimary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                }

                Text(skill.body)
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textSecondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(skill.frontmatter.name)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(FabushiAgentStudioStrings.text("m4.common.done", locale: locale)) {
                    inspectedSkill = nil
                }
            }
        }
    }

    @ViewBuilder
    private func publishPreflightSheet(app: FabushiStudioAutomationScriptApp, locale: FabushiLocale) -> some View {
        let result = FabushiStudioScriptAppPreflightValidator.validate(app)
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Image(systemName: result.isValid ? "checkmark.seal.fill" : "xmark.shield.fill")
                        .foregroundStyle(result.isValid ? theme.successColor : theme.destructiveColor)
                        .accessibilityHidden(true)
                    Text(
                        FabushiAgentStudioStrings.text(
                            result.isValid ? "m4.studio.skills.preflightPassed" : "m4.studio.skills.preflightFailed",
                            locale: locale
                        )
                    )
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    (result.isValid ? theme.successColor : theme.destructiveColor).opacity(0.12),
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )

                ForEach(result.issues) { issue in
                    VStack(alignment: .leading, spacing: 4) {
                        Text("\(issue.kind.rawValue) · \(issue.path)")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.destructiveColor)
                        Text(issue.message)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textPrimary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiAgentStudioStrings.text("m4.studio.skills.preflightTitle", locale: locale))
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(FabushiAgentStudioStrings.text("m4.common.done", locale: locale)) {
                    preflightSheetApp = nil
                }
            }
        }
    }
}

// MARK: - Tab 4: Memory (`MEMORY.md` 2200c / `USER.md` 1375c), `SOUL.md`, Honcho & Quarantine

@MainActor
internal struct FabushiStudioMemoryTabView: View {
    private enum Segment: String, CaseIterable, Identifiable {
        case memory, user, soul, honcho, quarantine
        var id: String { rawValue }
    }

    @Bindable var viewModel: FabushiAgentStudioViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var segment: Segment = .memory
    @State private var newEntryText = ""
    @State private var soulDraft = ""
    @State private var honchoQuestion = ""

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 14) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    segmentPill(.memory, key: "m4.studio.memory.segmentMemory", locale: locale)
                    segmentPill(.user, key: "m4.studio.memory.segmentUser", locale: locale)
                    segmentPill(.soul, key: "m4.studio.memory.segmentSoul", locale: locale)
                    segmentPill(.honcho, key: "m4.studio.memory.segmentHoncho", locale: locale)
                    segmentPill(.quarantine, key: "m4.studio.memory.segmentQuarantine", locale: locale)
                }
            }

            switch segment {
            case .memory:
                boundedMemoryEditor(target: .memory, state: viewModel.memoryState, locale: locale)
            case .user:
                boundedMemoryEditor(target: .user, state: viewModel.userMemoryState, locale: locale)
            case .soul:
                soulEditor(locale: locale)
            case .honcho:
                honchoInspector(locale: locale)
            case .quarantine:
                quarantineList(locale: locale)
            }
        }
        .onAppear {
            if soulDraft.isEmpty, let raw = viewModel.soulProfile?.rawMarkdown {
                soulDraft = raw
            }
        }
        .onChange(of: viewModel.soulProfile?.rawMarkdown) { _, updated in
            if let updated, soulDraft.isEmpty {
                soulDraft = updated
            }
        }
    }

    @ViewBuilder
    private func segmentPill(_ seg: Segment, key: String, locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text(key, locale: locale),
            accessibilityID: "fabushi.internal.studio.memory.segment.\(seg.rawValue)",
            isProminent: segment == seg
        ) {
            segment = seg
        }
    }

    @ViewBuilder
    private func boundedMemoryEditor(
        target: FabushiStudioMemoryTarget,
        state: FabushiStudioMemoryReadDTO?,
        locale: FabushiLocale
    ) -> some View {
        let used = state?.charCount ?? 0
        let limit = state?.charLimit ?? target.charLimit
        let remaining = state?.remainingChars ?? max(0, limit - used)

        VStack(alignment: .leading, spacing: 10) {
            ProgressView(value: Double(min(used, limit)), total: Double(max(1, limit)))
                .tint(used > Int(Double(limit) * 0.85) ? theme.warningColor : theme.accentColor)

            Text(
                FabushiAgentStudioStrings.format(
                    "m4.studio.memory.budgetFormat",
                    locale: locale,
                    Int32(clamping: used),
                    Int32(clamping: limit),
                    Int32(clamping: remaining)
                )
            )
            .fabushiFont(.caption)
            .foregroundStyle(theme.textSecondary)

            TextField(
                FabushiAgentStudioStrings.text("m4.studio.memory.addEntryPlaceholder", locale: locale),
                text: $newEntryText
            )
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            HStack(spacing: 8) {
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.studio.memory.addEntryButton", locale: locale),
                    systemImage: "plus",
                    accessibilityID: "fabushi.internal.studio.memory.addButton",
                    isProminent: true
                ) {
                    let entry = newEntryText
                    Task { @MainActor in
                        _ = await viewModel.mutateMemory(target: target, op: "add", entry: entry)
                        newEntryText = ""
                    }
                }
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.studio.memory.clearEntries", locale: locale),
                    systemImage: "trash",
                    accessibilityID: "fabushi.internal.studio.memory.clearButton",
                    isProminent: false
                ) {
                    Task { @MainActor in
                        _ = await viewModel.mutateMemory(target: target, op: "clear")
                    }
                }
            }

            ForEach(Array((state?.entries ?? []).enumerated()), id: \.offset) { _, entry in
                HStack(alignment: .top, spacing: 8) {
                    Text(entry)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textPrimary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Button(role: .destructive) {
                        Task { @MainActor in
                            _ = await viewModel.mutateMemory(target: target, op: "remove", matchSubstr: entry)
                        }
                    } label: {
                        Image(systemName: "minus.circle")
                            .foregroundStyle(theme.destructiveColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.delete", locale: locale))
                }
                .padding(10)
                .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
        }
    }

    @ViewBuilder
    private func soulEditor(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if let soul = viewModel.soulProfile {
                Text("\(soul.name) · \(soul.role) (\(soul.tone))")
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
            }

            TextEditor(text: $soulDraft)
                .frame(minHeight: 160)
                .padding(8)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            FabushiGlassPillButton(
                title: FabushiAgentStudioStrings.text("m4.studio.memory.soulSaveButton", locale: locale),
                systemImage: "checkmark.circle",
                accessibilityID: "fabushi.internal.studio.soulSaveButton",
                isProminent: true
            ) {
                let draft = soulDraft
                Task { @MainActor in
                    _ = await viewModel.saveSoul(markdown: draft)
                }
            }
        }
    }

    @ViewBuilder
    private func honchoInspector(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if let profile = viewModel.honchoProfile, !profile.summary.isEmpty {
                Text(profile.summary)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textPrimary)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            } else {
                Text(FabushiAgentStudioStrings.text("m4.studio.memory.honchoEmpty", locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
            }

            TextField(
                FabushiAgentStudioStrings.text("m4.studio.memory.honchoQueryPlaceholder", locale: locale),
                text: $honchoQuestion
            )
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            FabushiGlassPillButton(
                title: FabushiAgentStudioStrings.text("m4.studio.memory.honchoAskButton", locale: locale),
                systemImage: "sparkles",
                accessibilityID: "fabushi.internal.studio.honchoQueryButton",
                isProminent: true
            ) {
                let q = honchoQuestion
                Task { @MainActor in
                    _ = await viewModel.queryHoncho(question: q)
                }
            }

            if let result = viewModel.lastHonchoQueryResult {
                Text(result.answer)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textPrimary)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
        }
    }

    @ViewBuilder
    private func quarantineList(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(FabushiAgentStudioStrings.text("m4.studio.memory.quarantineHostNote", locale: locale))
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            if viewModel.quarantineProposals.isEmpty {
                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.studio.memory.quarantineEmptyTitle", locale: locale),
                    systemImage: "shield.checkered",
                    descriptionText: FabushiAgentStudioStrings.text("m4.studio.memory.quarantineEmptyDesc", locale: locale),
                    accessibilityID: "fabushi.internal.studio.emptyQuarantine"
                )
            } else {
                ForEach(viewModel.quarantineProposals) { proposal in
                    VStack(alignment: .leading, spacing: 6) {
                        Text("\(proposal.proposedBy) → \(proposal.target)")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textMuted)
                        Text(proposal.content)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textPrimary)
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.studio.memory.quarantinePromote", locale: locale),
                            systemImage: "arrow.up.doc",
                            accessibilityID: "fabushi.internal.studio.quarantine.promote.\(proposal.id)",
                            isProminent: true
                        ) {
                            Task { @MainActor in
                                _ = await viewModel.promoteQuarantineProposal(proposal)
                            }
                        }
                    }
                    .padding(12)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
        }
    }
}

// MARK: - Tab 5: 5-Field Cron Schedules & 7-Day Preview

@MainActor
internal struct FabushiStudioCronTabView: View {
    @Bindable var viewModel: FabushiAgentStudioViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var cronName = ""
    @State private var cronExpression = "0 9 * * 1-5"
    @State private var cronPrompt = ""

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 10) {
                FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.cron.newJob", locale: locale))

                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.cron.namePlaceholder", locale: locale),
                    text: $cronName
                )
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.cron.exprPlaceholder", locale: locale),
                    text: $cronExpression
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.cron.promptPlaceholder", locale: locale),
                    text: $cronPrompt
                )
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                HStack(spacing: 8) {
                    FabushiGlassPillButton(
                        title: FabushiAgentStudioStrings.text("m4.studio.cron.previewButton", locale: locale),
                        systemImage: "calendar.badge.clock",
                        accessibilityID: "fabushi.internal.studio.cronPreviewButton",
                        isProminent: false
                    ) {
                        let expr = cronExpression
                        Task { @MainActor in
                            _ = await viewModel.previewCron(schedule: expr)
                        }
                    }

                    FabushiGlassPillButton(
                        title: FabushiAgentStudioStrings.text("m4.common.save", locale: locale),
                        systemImage: "plus.circle",
                        accessibilityID: "fabushi.internal.studio.cronSaveButton",
                        isProminent: true
                    ) {
                        let name = cronName.isEmpty ? "Scheduled Routine" : cronName
                        let expr = cronExpression
                        let prompt = cronPrompt.isEmpty ? "Summarize daily updates" : cronPrompt
                        Task { @MainActor in
                            if await viewModel.saveCronJob(name: name, schedule: expr, prompt: prompt) != nil {
                                cronName = ""
                                cronPrompt = ""
                            }
                        }
                    }
                }

                if let preview = viewModel.cronPreview {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(
                            FabushiAgentStudioStrings.format(
                                "m4.studio.cron.previewFiresFormat",
                                locale: locale,
                                Int32(clamping: preview.summary.scheduledFiresNext7Days)
                            )
                        )
                        .fabushiFont(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.successColor)

                        if let nextRun = preview.nextRun {
                            Text("\(FabushiAgentStudioStrings.text("m4.studio.cron.previewNextRun", locale: locale)): \(nextRun)")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.successColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))

            if viewModel.cronJobs.isEmpty {
                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.studio.cron.emptyTitle", locale: locale),
                    systemImage: "clock.arrow.2.circlepath",
                    descriptionText: FabushiAgentStudioStrings.text("m4.studio.cron.emptyDesc", locale: locale),
                    accessibilityID: "fabushi.internal.studio.emptyCron"
                )
            } else {
                ForEach(viewModel.cronJobs) { job in
                    HStack(alignment: .top, spacing: 10) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(job.name)
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Text(job.schedule)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.accentColor)
                            Text(job.prompt)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(2)
                        }
                        Spacer()
                        Button(role: .destructive) {
                            Task { @MainActor in
                                _ = await viewModel.deleteCronJob(jobId: job.id)
                            }
                        } label: {
                            Image(systemName: "trash")
                                .foregroundStyle(theme.destructiveColor)
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.delete", locale: locale))
                    }
                    .padding(12)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                }
            }
        }
    }
}

// MARK: - Tab 6: Value-Bound Approvals & Browser Takeover Dock (`D11-03`)

@MainActor
internal struct FabushiStudioApprovalsTabView: View {
    @Bindable var viewModel: FabushiAgentStudioViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var vbAction = "file_write"
    @State private var vbTarget = "/workspace/README.md"

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 16) {
            // Tool Approval Dock (`fabushi.stage.toolApprovalDock`)
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.approvals.dockTitle", locale: locale))
                    Spacer()
                    Toggle(
                        FabushiAgentStudioStrings.text("m4.studio.approvals.pendingOnlyToggle", locale: locale),
                        isOn: $viewModel.approvalsPendingOnly
                    )
                    .labelsHidden()
                    .accessibilityLabel(FabushiAgentStudioStrings.text("m4.studio.approvals.pendingOnlyToggle", locale: locale))
                    .onChange(of: viewModel.approvalsPendingOnly) { _, newValue in
                        Task { @MainActor in
                            await viewModel.loadApprovals(pendingOnly: newValue)
                        }
                    }
                }

                ForEach(Array(viewModel.liveToolApprovalRequests.enumerated()), id: \.element.id) { idx, req in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(req.toolName)
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.textPrimary)
                        Text(req.summary)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textSecondary)
                        HStack(spacing: 8) {
                            FabushiGlassPillButton(
                                title: FabushiAgentStudioStrings.text("m4.studio.approvals.approveOnce", locale: locale),
                                systemImage: "checkmark",
                                accessibilityID: idx == 0
                                    ? FabushiAccessibilityIDs.Stage.toolApproveButton
                                    : "fabushi.internal.studio.liveApprove.\(req.id)",
                                isProminent: true
                            ) {
                                Task { @MainActor in
                                    _ = await viewModel.resolveToolDock(requestId: req.approvalId, approved: true)
                                }
                            }
                            FabushiGlassPillButton(
                                title: FabushiAgentStudioStrings.text("m4.studio.approvals.deny", locale: locale),
                                systemImage: "xmark",
                                accessibilityID: idx == 0
                                    ? FabushiAccessibilityIDs.Stage.toolDenyButton
                                    : "fabushi.internal.studio.liveDeny.\(req.id)",
                                isProminent: false
                            ) {
                                Task { @MainActor in
                                    _ = await viewModel.resolveToolDock(requestId: req.approvalId, approved: false)
                                }
                            }
                        }
                    }
                    .padding(12)
                    .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }

                if viewModel.approvals.isEmpty && viewModel.liveToolApprovalRequests.isEmpty {
                    FabushiEmptyStateView(
                        title: FabushiAgentStudioStrings.text("m4.studio.approvals.emptyTitle", locale: locale),
                        systemImage: "checkmark.shield",
                        descriptionText: FabushiAgentStudioStrings.text("m4.studio.approvals.emptyDesc", locale: locale),
                        accessibilityID: "fabushi.internal.studio.emptyApprovals"
                    )
                } else {
                    let hasLive = !viewModel.liveToolApprovalRequests.isEmpty
                    ForEach(Array(viewModel.approvals.enumerated()), id: \.element.id) { idx, rec in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Text(rec.command)
                                    .fabushiFont(.monospacedCaption)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(theme.textPrimary)
                                    .lineLimit(2)
                                Spacer()
                                Text(rec.status.rawValue)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(rec.status == .pending ? theme.warningColor : theme.textSecondary)
                            }
                            Text("\(rec.riskLevel) · \(rec.patternName)")
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textMuted)

                            if rec.status == .pending {
                                let useCanonicalApproveIds = !hasLive && idx == 0
                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 8) {
                                        FabushiGlassPillButton(
                                            title: FabushiAgentStudioStrings.text("m4.studio.approvals.approveOnce", locale: locale),
                                            accessibilityID: useCanonicalApproveIds
                                                ? FabushiAccessibilityIDs.Stage.toolApproveButton
                                                : "fabushi.internal.studio.approveOnce.\(rec.id)",
                                            isProminent: true
                                        ) {
                                            Task { @MainActor in
                                                _ = await viewModel.decideApproval(approvalId: rec.id, decision: .approvedOnce)
                                            }
                                        }
                                        FabushiGlassPillButton(
                                            title: FabushiAgentStudioStrings.text("m4.studio.approvals.approveSession", locale: locale),
                                            accessibilityID: "fabushi.internal.studio.approveSession.\(rec.id)",
                                            isProminent: false
                                        ) {
                                            Task { @MainActor in
                                                _ = await viewModel.decideApproval(approvalId: rec.id, decision: .approvedSession)
                                            }
                                        }
                                        FabushiGlassPillButton(
                                            title: FabushiAgentStudioStrings.text("m4.studio.approvals.deny", locale: locale),
                                            accessibilityID: useCanonicalApproveIds
                                                ? FabushiAccessibilityIDs.Stage.toolDenyButton
                                                : "fabushi.internal.studio.deny.\(rec.id)",
                                            isProminent: false
                                        ) {
                                            Task { @MainActor in
                                                _ = await viewModel.decideApproval(approvalId: rec.id, decision: .denied)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        .padding(12)
                        .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.toolApprovalDock)

            // Browser Takeover Controls (`D11-03`)
            VStack(alignment: .leading, spacing: 10) {
                FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.approvals.takeoverSection", locale: locale))

                if viewModel.browserTakeoverRequests.isEmpty {
                    Text(FabushiAgentStudioStrings.text("m4.studio.approvals.takeoverIdle", locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(viewModel.browserTakeoverRequests) { req in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(req.sessionId)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.accentColor)
                            Text(req.reason)
                                .fabushiFont(.footnote)
                                .foregroundStyle(theme.textPrimary)
                            HStack(spacing: 8) {
                                FabushiGlassPillButton(
                                    title: FabushiAgentStudioStrings.text("m4.studio.approvals.takeOverButton", locale: locale),
                                    accessibilityID: "fabushi.internal.studio.takeover.\(req.sessionId)",
                                    isProminent: true
                                ) {
                                    Task { @MainActor in
                                        await viewModel.handleBrowserTakeoverAction(sessionId: req.sessionId, action: .takeOver)
                                    }
                                }
                                FabushiGlassPillButton(
                                    title: FabushiAgentStudioStrings.text("m4.studio.approvals.resumeAiButton", locale: locale),
                                    accessibilityID: "fabushi.internal.studio.resumeAi.\(req.sessionId)",
                                    isProminent: false
                                ) {
                                    Task { @MainActor in
                                        await viewModel.handleBrowserTakeoverAction(sessionId: req.sessionId, action: .resumeAi)
                                    }
                                }
                                FabushiGlassPillButton(
                                    title: FabushiAgentStudioStrings.text("m4.studio.approvals.cancelAiButton", locale: locale),
                                    accessibilityID: "fabushi.internal.studio.cancelAi.\(req.sessionId)",
                                    isProminent: false
                                ) {
                                    Task { @MainActor in
                                        await viewModel.handleBrowserTakeoverAction(sessionId: req.sessionId, action: .cancelAi)
                                    }
                                }
                            }
                        }
                        .padding(10)
                        .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))

            // Single-Use SHA-256 Value-Bound Approval Ledger
            VStack(alignment: .leading, spacing: 10) {
                FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.approvals.valueBoundSection", locale: locale))

                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.approvals.actionPlaceholder", locale: locale),
                    text: $vbAction
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.approvals.targetPlaceholder", locale: locale),
                    text: $vbTarget
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.studio.approvals.issueTokenButton", locale: locale),
                    systemImage: "key.fill",
                    accessibilityID: "fabushi.internal.studio.issueValueBoundButton",
                    isProminent: true
                ) {
                    let action = vbAction
                    let target = vbTarget
                    Task { @MainActor in
                        _ = await viewModel.issueValueBoundApproval(action: action, target: target)
                    }
                }

                ForEach(viewModel.valueBoundTokens) { token in
                    HStack {
                        VStack(alignment: .leading, spacing: 3) {
                            Text("\(token.action) → \(token.target)")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textPrimary)
                            Text(token.approvalId)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                        }
                        Spacer()
                        if token.spent {
                            Text(FabushiAgentStudioStrings.text("m4.studio.approvals.spentBadge", locale: locale))
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                        } else {
                            Button(FabushiAgentStudioStrings.text("m4.studio.approvals.consumeTokenButton", locale: locale)) {
                                Task { @MainActor in
                                    _ = await viewModel.consumeValueBoundApproval(
                                        approvalId: token.approvalId,
                                        action: token.action,
                                        target: token.target
                                    )
                                }
                            }
                            .fabushiFont(.caption)
                            .fabushiMinimumTouchTarget()
                        }
                    }
                    .padding(10)
                    .background(theme.surfacePrimary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }
}

// MARK: - Tab 7: Sessions, FTS5 Search & `D11-05` Redacted Trajectory Inspector

@MainActor
internal struct FabushiStudioSessionsTabView: View {
    @Bindable var viewModel: FabushiAgentStudioViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    @State private var searchQuery = ""
    @State private var newSessionTitle = ""

    var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 14) {
            // D11-05 Active Secret & JWT Redaction Badge
            HStack(spacing: 8) {
                Image(systemName: "eye.slash.fill")
                    .foregroundStyle(theme.successColor)
                    .accessibilityHidden(true)
                Text(FabushiAgentStudioStrings.text("m4.studio.sessions.redactionActiveBadge", locale: locale))
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.successColor)
                Spacer()
                if viewModel.lastSessionRedactionCount > 0 {
                    Text(
                        FabushiAgentStudioStrings.format(
                            "m4.studio.sessions.redactionCountFormat",
                            locale: locale,
                            Int32(clamping: viewModel.lastSessionRedactionCount)
                        )
                    )
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.textSecondary)
                }
            }
            .padding(10)
            .background(theme.successColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            // FTS5 / CJK Search Bar
            HStack(spacing: 8) {
                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.sessions.searchPlaceholder", locale: locale),
                    text: $searchQuery
                )
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .onSubmit {
                    Task { @MainActor in await viewModel.searchSessions(query: searchQuery) }
                }
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.common.search", locale: locale),
                    systemImage: "magnifyingglass",
                    accessibilityID: "fabushi.internal.studio.sessionSearchButton",
                    isProminent: false
                ) {
                    Task { @MainActor in await viewModel.searchSessions(query: searchQuery) }
                }
            }

            // Create Local Session
            HStack(spacing: 8) {
                TextField(
                    FabushiAgentStudioStrings.text("m4.studio.sessions.titlePlaceholder", locale: locale),
                    text: $newSessionTitle
                )
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.studio.sessions.newSession", locale: locale),
                    systemImage: "plus",
                    accessibilityID: "fabushi.internal.studio.newSessionButton",
                    isProminent: true
                ) {
                    let title = newSessionTitle.isEmpty ? "Studio Session" : newSessionTitle
                    Task { @MainActor in
                        if await viewModel.createSession(title: title) != nil {
                            newSessionTitle = ""
                        }
                    }
                }
            }

            if !viewModel.sessionSearchHits.isEmpty {
                FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.studio.sessions.searchResultsTitle", locale: locale))
                ForEach(viewModel.sessionSearchHits) { hit in
                    VStack(alignment: .leading, spacing: 4) {
                        Text("\(hit.role) · \(hit.backend)")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.accentColor)
                        Text(hit.snippet)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textPrimary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }

            if viewModel.sessions.isEmpty {
                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.studio.sessions.emptyTitle", locale: locale),
                    systemImage: "text.magnifyingglass",
                    descriptionText: FabushiAgentStudioStrings.text("m4.studio.sessions.emptyDesc", locale: locale),
                    accessibilityID: "fabushi.internal.studio.emptySessions"
                )
            } else {
                ForEach(viewModel.sessions) { session in
                    Button {
                        Task { @MainActor in _ = await viewModel.inspectSession(sessionId: session.id) }
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text({
                                    if let title = session.title, !title.isEmpty { return title }
                                    return session.id
                                }())
                                    .fabushiFont(.subheadline)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(theme.textPrimary)
                                Spacer()
                                Text(session.model)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.accentColor)
                            }
                            Text(
                                FabushiAgentStudioStrings.format(
                                    "m4.studio.sessions.tokensFormat",
                                    locale: locale,
                                    Int32(clamping: session.inputTokens),
                                    Int32(clamping: session.outputTokens)
                                )
                            )
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                    }
                    .buttonStyle(.plain)
                }
            }

            if let inspected = viewModel.inspectedSession {
                VStack(alignment: .leading, spacing: 6) {
                    Text(inspected.title ?? inspected.id)
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                    Text(viewModel.inspectedSessionRedactedMetadata)
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(theme.textSecondary)
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
        }
    }
}
