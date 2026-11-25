import Foundation
import SwiftUI

/// Root 7-tab Adaptive Agent Studio workspace (`FabushiAccessibilityIDs.Studio.root`).
///
/// Rendered directly inside `FabushiAdaptiveShellView`'s `NavigationStack` on both iPad and iPhone
/// (strictly ZERO nested `NavigationStack` on root). Accepts optional pluggable builders for the
/// Official OAuth 2.1 MCP Console and Coding Agents Panel so M4b integrates cleanly without
/// refactoring Studio layout.
@MainActor
public struct FabushiAgentStudioWorkspaceView: View {
    public typealias AuxiliaryConsoleBuilder = @MainActor (FabushiCoordinator, ShellNavigationRouter) -> AnyView

    public let initialTab: String?
    private let mcpConsoleBuilder: AuxiliaryConsoleBuilder?
    private let codingAgentsPanelBuilder: AuxiliaryConsoleBuilder?

    @State private var viewModel: FabushiAgentStudioViewModel
    @State private var isMcpSheetPresented = false
    @State private var isMultiAgentLabPresented = false
    @State private var hasAppliedInitialTab = false

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        initialTab: String? = nil,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub(),
        mcpConsoleBuilder: AuxiliaryConsoleBuilder? = nil,
        codingAgentsPanelBuilder: AuxiliaryConsoleBuilder? = nil
    ) {
        self.initialTab = initialTab
        self.mcpConsoleBuilder = mcpConsoleBuilder
        self.codingAgentsPanelBuilder = codingAgentsPanelBuilder
        self._viewModel = State(
            initialValue: FabushiAgentStudioViewModel(
                coordinator: coordinator,
                router: router,
                cleanupHub: cleanupHub
            )
        )
    }

    public var body: some View {
        let locale = theme.locale
        let isRegularWidth = horizontalSizeClass == .regular
        let reduceMotion = theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)

        VStack(spacing: 0) {
            tabBarStrip(locale: locale, reduceMotion: reduceMotion)
            Divider().overlay(theme.separatorHairline)

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let classified = viewModel.lastClassifiedError {
                        FabushiStudioClassifiedErrorBanner(
                            error: classified,
                            onOpenAiProviders: {
                                viewModel.router.navigate(
                                    to: .settings(section: "aiProviders"),
                                    coordinator: viewModel.coordinator
                                )
                            },
                            onDismiss: {
                                viewModel.dismissClassifiedError()
                            }
                        )
                    }

                    activeTabContent(isRegularWidth: isRegularWidth, locale: locale)
                }
                .padding(16)
            }
            .background(theme.surfacePrimary)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiAgentStudioStrings.text("m4.studio.title", locale: locale))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { @MainActor in
                        await viewModel.refreshActiveTab()
                    }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .fabushiMinimumTouchTarget()
                .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.refresh", locale: locale))
                .accessibilityIdentifier("fabushi.internal.studio.refreshButton")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.root)
        .onAppear {
            if !hasAppliedInitialTab {
                hasAppliedInitialTab = true
                applyDeepLinkTab(initialTab)
            }
        }
        .onChange(of: initialTab) { _, updatedTab in
            applyDeepLinkTab(updatedTab)
        }
        .task(id: "\(viewModel.selectedTab.rawValue):\(viewModel.coordinator.accountScopeGeneration)") {
            await viewModel.refreshActiveTab()
        }
        .onChange(of: viewModel.coordinator.accountScopeGeneration) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .onChange(of: viewModel.coordinator.activeAccountId) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .sheet(isPresented: $isMcpSheetPresented) {
            NavigationStack {
                if let mcpConsoleBuilder {
                    mcpConsoleBuilder(viewModel.coordinator, viewModel.router)
                        .toolbar {
                            ToolbarItem(placement: .topBarLeading) {
                                Button(FabushiAgentStudioStrings.text("m4.common.close", locale: locale)) {
                                    isMcpSheetPresented = false
                                }
                                .fabushiMinimumTouchTarget()
                            }
                        }
                } else {
                    FabushiStudioDefaultMcpFallbackView(
                        locale: locale,
                        onClose: { isMcpSheetPresented = false }
                    )
                }
            }
            .environment(theme)
        }
        .sheet(isPresented: $isMultiAgentLabPresented) {
            NavigationStack {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        FabushiMultiAgentRoomAccessoriesView(
                            chatId: 0,
                            coordinator: viewModel.coordinator,
                            router: viewModel.router,
                            cleanupHub: viewModel.cleanupHub,
                            isChatStageSurface: false
                        )
                    }
                    .padding(16)
                }
                .background(theme.surfacePrimary)
                .navigationTitle(FabushiAgentStudioStrings.text("m4.studio.overview.openMultiAgent", locale: locale))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button(FabushiAgentStudioStrings.text("m4.common.close", locale: locale)) {
                            isMultiAgentLabPresented = false
                        }
                        .fabushiMinimumTouchTarget()
                    }
                }
            }
            .environment(theme)
        }
    }

    // MARK: - 7-Tab Selector Strip

    @ViewBuilder
    private func tabBarStrip(locale: FabushiLocale, reduceMotion: Bool) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(FabushiStudioTab.allCases) { tab in
                    let isSelected = viewModel.selectedTab == tab
                    let label = FabushiAgentStudioStrings.text(tab.titleKey, locale: locale)

                    Button {
                        if reduceMotion {
                            viewModel.selectedTab = tab
                        } else {
                            withAnimation(.easeInOut(duration: 0.18)) {
                                viewModel.selectedTab = tab
                            }
                        }
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: tab.systemImage)
                                .accessibilityHidden(true)
                            Text(label)
                                .fabushiFont(.callout)
                                .fontWeight(isSelected ? .semibold : .regular)
                                .lineLimit(1)
                        }
                        .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textPrimary)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .fabushiMinimumTouchTarget()
                        .background {
                            if isSelected {
                                Capsule().fill(theme.accentColor)
                            } else {
                                Capsule().fill(theme.surfaceSecondary)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(label)
                    .accessibilityAddTraits(isSelected ? [.isSelected] : [])
                    .accessibilityIdentifier(tab.accessibilityIdentifier)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .background(theme.surfaceTopBar)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(FabushiAgentStudioStrings.text("m4.studio.tabBarLabel", locale: locale))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.tabBar)
    }

    // MARK: - Active Tab Content Router

    @ViewBuilder
    private func activeTabContent(isRegularWidth: Bool, locale: FabushiLocale) -> some View {
        switch viewModel.selectedTab {
        case .overview:
            let codingPanel: AnyView = codingAgentsPanelBuilder?(viewModel.coordinator, viewModel.router)
                ?? AnyView(FabushiStudioDefaultCodingFallbackView(locale: locale))
            FabushiStudioOverviewTabView(
                viewModel: viewModel,
                isRegularWidth: isRegularWidth,
                onOpenMcpConsole: { isMcpSheetPresented = true },
                onOpenMultiAgentLab: { isMultiAgentLabPresented = true },
                codingAgentsPanelContent: codingPanel
            )
        case .kanban:
            FabushiStudioKanbanTabView(viewModel: viewModel, isRegularWidth: isRegularWidth)
        case .skills:
            FabushiStudioSkillsTabView(viewModel: viewModel)
        case .memory:
            FabushiStudioMemoryTabView(viewModel: viewModel)
        case .cron:
            FabushiStudioCronTabView(viewModel: viewModel)
        case .approvals:
            FabushiStudioApprovalsTabView(viewModel: viewModel)
        case .sessions:
            FabushiStudioSessionsTabView(viewModel: viewModel)
        }
    }

    private func applyDeepLinkTab(_ rawTab: String?) {
        if let mapped = FabushiStudioTab.fromDeepLinkAlias(rawTab) {
            viewModel.selectedTab = mapped
        }
        if FabushiStudioTab.auxiliarySheetTarget(from: rawTab) == "mcp" {
            isMcpSheetPresented = true
        }
    }
}

// MARK: - Standalone Safe Fallbacks (Replaced by Worker 2's M4b Concrete Panels via Builders)

@MainActor
private struct FabushiStudioDefaultCodingFallbackView: View {
    let locale: FabushiLocale

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.coding.title", locale: locale))
            Text(FabushiAgentStudioStrings.text("m4.coding.remoteOnlyNotice", locale: locale))
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.codingAgentsPanel)
    }
}

@MainActor
private struct FabushiStudioDefaultMcpFallbackView: View {
    let locale: FabushiLocale
    let onClose: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(FabushiAgentStudioStrings.text("m4.mcp.stdioBlockedNotice", locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)

                FabushiEmptyStateView(
                    title: FabushiAgentStudioStrings.text("m4.mcp.auditSection", locale: locale),
                    systemImage: "list.bullet.rectangle.portrait",
                    descriptionText: FabushiAgentStudioStrings.text("m4.mcp.auditEmpty", locale: locale),
                    accessibilityID: FabushiAccessibilityIDs.Studio.mcpAuditLogList
                )
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiAgentStudioStrings.text("m4.mcp.title", locale: locale))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button(FabushiAgentStudioStrings.text("m4.common.close", locale: locale), action: onClose)
                    .fabushiMinimumTouchTarget()
            }
        }
    }
}
