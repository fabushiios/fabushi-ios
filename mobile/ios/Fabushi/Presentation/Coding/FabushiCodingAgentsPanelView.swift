import Foundation
import SwiftUI

/// Agent Studio Coding Agents Panel (`FabushiAccessibilityIDs.Studio.codingAgentsPanel`).
///
/// Embedded directly inside `FabushiAgentStudioWorkspaceView` Overview (ZERO nested root `NavigationStack`).
/// Supports remote coding host probing, BotFather NL coding-bot creation, 6-provider catalog,
/// permission mode controls, worktree/branch thread persistence, checkpoints, and PR dispatch.
@MainActor
public struct FabushiCodingAgentsPanelView: View {
    @State private var viewModel: FabushiCodingViewModel
    @State private var isNewBotSheetPresented: Bool = false
    @State private var isPullRequestSheetPresented: Bool = false

    // New Coding Bot sheet fields
    @State private var draftBotName: String = ""
    @State private var draftProviderKind: FabushiCodingProviderKind = .claudeCode
    @State private var draftWorkspacePath: String = "/workspace"
    @State private var draftBranchName: String = "main"
    @State private var draftUseWorktree: Bool = true
    @State private var draftPermissionMode: FabushiCodingPermissionMode = .ask

    // PR sheet fields
    @State private var draftPrTitle: String = ""
    @State private var draftPrBody: String = ""

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self._viewModel = State(
            initialValue: FabushiCodingViewModel(
                chatId: 0,
                coordinator: coordinator,
                router: router,
                cleanupHub: cleanupHub
            )
        )
    }

    public var body: some View {
        let locale = theme.locale

        VStack(alignment: .leading, spacing: 14) {
            // Header + Account Isolation (`D10-02`) Badge
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.coding.title", locale: locale))
                    Spacer(minLength: 4)
                    isolationBadge(locale: locale)
                }
                VStack(alignment: .leading, spacing: 6) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.coding.title", locale: locale))
                    isolationBadge(locale: locale)
                }
            }

            // Explicit Remote-Only iOS Sandbox Boundary Notice (`IOS-MAIN-INV-04`)
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "desktopcomputer.and.arrow.down")
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)
                Text(viewModel.lastSandboxNotice ?? FabushiAgentStudioStrings.text("m4.coding.remoteOnlyNotice", locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            // Primary Action Controls: Probe Remote Coding Host + Create PR + New Bot
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    probeHostButton(locale: locale)
                    createPrButton(locale: locale)
                    newBotButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    probeHostButton(locale: locale)
                    createPrButton(locale: locale)
                    newBotButton(locale: locale)
                }
            }

            if !viewModel.lastRemoteHostProbeSummary.isEmpty {
                Text(viewModel.lastRemoteHostProbeSummary)
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }

            // Remote Coding Hosts (`computers.list`)
            if !viewModel.remoteComputers.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text(FabushiAgentStudioStrings.text("m4.coding.remoteHostsSection", locale: locale))
                        .fabushiFont(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.textPrimary)

                    ForEach(viewModel.remoteComputers) { host in
                        HStack(spacing: 8) {
                            Circle()
                                .fill(host.online ? theme.successColor : theme.textMuted)
                                .frame(width: 8, height: 8)
                                .accessibilityHidden(true)
                            Text(host.displayName)
                                .fabushiFont(.callout)
                                .foregroundStyle(theme.textPrimary)
                            Spacer(minLength: 4)
                            Text(host.ipAddress)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                        .padding(8)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                }
            }

            // 6-Provider Catalog (`coding.listProvidersCatalog`)
            VStack(alignment: .leading, spacing: 6) {
                Text(FabushiAgentStudioStrings.text("m4.coding.providersSection", locale: locale))
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(
                            viewModel.providersCatalog.isEmpty
                                ? FabushiCodingProviderKind.allCases.map {
                                    FabushiCodingProviderCatalogItemDTO(kind: $0, supportsAcp: $0.supportsAcp)
                                }
                                : viewModel.providersCatalog
                        ) { item in
                            HStack(spacing: 5) {
                                Text(item.kind.displayName)
                                    .fabushiFont(.caption)
                                    .fontWeight(.medium)
                                    .foregroundStyle(theme.textPrimary)
                                if item.supportsAcp {
                                    Text(FabushiAgentStudioStrings.text("m4.coding.acpBadge", locale: locale))
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.accentForegroundColor)
                                        .padding(.horizontal, 6)
                                        .padding(.vertical, 2)
                                        .background(theme.accentColor, in: Capsule())
                                }
                            }
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(theme.surfaceElevated, in: Capsule())
                        }
                    }
                }
            }

            // BotFather Natural Language Setup (`fabushi.botCreateNl`)
            VStack(alignment: .leading, spacing: 8) {
                Text(FabushiAgentStudioStrings.text("m4.coding.botfatherSection", locale: locale))
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)

                TextField(
                    FabushiAgentStudioStrings.text("m4.coding.botfatherPlaceholder", locale: locale),
                    text: $viewModel.botFatherPromptDraft,
                    axis: .vertical
                )
                .fabushiFont(.body)
                .lineLimit(2...4)
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("fabushi.internal.coding.botfatherPromptField")

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.coding.botfatherCreateButton", locale: locale),
                    systemImage: "sparkles",
                    accessibilityID: "fabushi.internal.coding.botfatherCreateButton",
                    isProminent: false
                ) {
                    Task { @MainActor in
                        _ = await viewModel.createBotViaBotFatherNL(prompt: viewModel.botFatherPromptDraft)
                    }
                }
                .disabled(viewModel.botFatherPromptDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

                if let summary = viewModel.lastBotFatherResponse?.summaryMarkdown, !summary.isEmpty {
                    Text(summary)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }

            // Permission Mode Selector (`ask` / `auto_edit` / `full_access`)
            Picker(
                FabushiAgentStudioStrings.text("m4.coding.permissionAsk", locale: locale),
                selection: $viewModel.selectedPermissionMode
            ) {
                ForEach(FabushiCodingPermissionMode.allCases) { mode in
                    Text(FabushiAgentStudioStrings.text(mode.localizationKey, locale: locale)).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("fabushi.internal.coding.permissionModePicker")

            // Configured Coding Bots (`coding.listBots`)
            VStack(alignment: .leading, spacing: 8) {
                Text(FabushiAgentStudioStrings.text("m4.coding.botsSection", locale: locale))
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)

                if viewModel.codingBots.isEmpty {
                    Text(FabushiAgentStudioStrings.text("m4.coding.noBots", locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(viewModel.codingBots) { bot in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 6) {
                                Text(bot.displayName)
                                    .fabushiFont(.callout)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(theme.textPrimary)
                                Text("@\(bot.username)")
                                    .fabushiFont(.monospacedCaption)
                                    .foregroundStyle(theme.textSecondary)
                                Spacer(minLength: 4)
                                if let cfg = bot.codingAgent {
                                    Text(cfg.providerKind.displayName)
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.accentColor)
                                }
                            }
                            if let cfg = bot.codingAgent {
                                Text("\(cfg.workspacePath) · \(FabushiAgentStudioStrings.text(cfg.permissionMode.localizationKey, locale: locale))")
                                    .fabushiFont(.monospacedCaption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            }

            if !viewModel.lastCommandOutput.isEmpty {
                Text(viewModel.lastCommandOutput)
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textSecondary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Studio.codingAgentsPanel)
        .task(id: viewModel.coordinator.accountScopeGeneration) {
            await viewModel.loadAll()
        }
        .onChange(of: viewModel.coordinator.accountScopeGeneration) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .onChange(of: viewModel.coordinator.activeAccountId) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .sheet(isPresented: $isNewBotSheetPresented) {
            newCodingBotSheet(locale: locale)
                .environment(theme)
        }
        .sheet(isPresented: $isPullRequestSheetPresented) {
            createPullRequestSheet(locale: locale)
                .environment(theme)
        }
    }

    // MARK: - Subviews

    @ViewBuilder
    private func isolationBadge(locale: FabushiLocale) -> some View {
        HStack(spacing: 4) {
            Image(systemName: "person.badge.shield.checkmark.fill")
                .accessibilityHidden(true)
            Text(FabushiAgentStudioStrings.text("m4.coding.accountIsolationBadge", locale: locale))
                .fabushiFont(.caption2)
                .fontWeight(.semibold)
        }
        .foregroundStyle(theme.accentColor)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(theme.accentColor.opacity(0.14), in: Capsule())
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
    private func newBotButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.coding.newBotButton", locale: locale),
            systemImage: "plus",
            accessibilityID: "fabushi.internal.coding.newBotButton",
            isProminent: false
        ) {
            isNewBotSheetPresented = true
        }
    }

    // MARK: - Modal Sheets (Wrapped in `NavigationStack`)

    @ViewBuilder
    private func newCodingBotSheet(locale: FabushiLocale) -> some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.coding.botNamePlaceholder", locale: locale),
                        text: $draftBotName
                    )
                    Picker("Provider", selection: $draftProviderKind) {
                        ForEach(FabushiCodingProviderKind.allCases) { kind in
                            Text(kind.displayName).tag(kind)
                        }
                    }
                    TextField(
                        FabushiAgentStudioStrings.text("m4.coding.workspacePlaceholder", locale: locale),
                        text: $draftWorkspacePath
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    TextField(
                        FabushiAgentStudioStrings.text("m4.coding.branchPlaceholder", locale: locale),
                        text: $draftBranchName
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    Toggle(
                        FabushiAgentStudioStrings.text("m4.coding.useWorktreeToggle", locale: locale),
                        isOn: $draftUseWorktree
                    )
                    Picker("Permission", selection: $draftPermissionMode) {
                        ForEach(FabushiCodingPermissionMode.allCases) { mode in
                            Text(FabushiAgentStudioStrings.text(mode.localizationKey, locale: locale)).tag(mode)
                        }
                    }
                }
            }
            .navigationTitle(FabushiAgentStudioStrings.text("m4.coding.newBotButton", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                        isNewBotSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(FabushiAgentStudioStrings.text("m4.common.create", locale: locale)) {
                        let req = FabushiCodingCreateBotRequest(
                            displayName: draftBotName,
                            username: nil,
                            codingAgent: FabushiCodingAgentBotConfigDTO(
                                providerKind: draftProviderKind,
                                workspacePath: draftWorkspacePath,
                                useWorktree: draftUseWorktree,
                                permissionMode: draftPermissionMode
                            )
                        )
                        Task { @MainActor in
                            _ = await viewModel.createCodingBot(req)
                            isNewBotSheetPresented = false
                        }
                    }
                    .disabled(draftBotName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }

    @ViewBuilder
    private func createPullRequestSheet(locale: FabushiLocale) -> some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.coding.prTitlePlaceholder", locale: locale),
                        text: $draftPrTitle
                    )
                    TextField(
                        FabushiAgentStudioStrings.text("m4.coding.prBodyPlaceholder", locale: locale),
                        text: $draftPrBody,
                        axis: .vertical
                    )
                    .lineLimit(3...6)
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
                        let cmd = draftPrTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? "/pr"
                            : "/pr \(draftPrTitle)"
                        Task { @MainActor in
                            _ = await viewModel.dispatchRemoteCodingCommand(cmd)
                            isPullRequestSheetPresented = false
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
