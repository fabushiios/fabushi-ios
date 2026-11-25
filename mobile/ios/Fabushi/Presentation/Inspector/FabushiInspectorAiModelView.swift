import Foundation
import SwiftUI

// MARK: - Bot Configuration & Dynamic AI Model Picker (`#17`, `D10-09`)

/// Pure state & validation helper for `D10-09` Bot AI Model Configuration:
/// - Populates choices strictly from live `FabushiAiModelCatalog` (`magpie.state(remote: true)` + offline fallback).
/// - Normalizes sentinel defaults back to `""` ("Follow Settings").
/// - Honors `updatedAtMs > 0` so clearing an override back to `""` never resurrects stale cached values.
/// - ZERO hardcoded model defaults anywhere.
public struct FabushiInspectorBotModelState: Sendable, Equatable {
    public var selectedPreferredModel: String
    public let catalog: FabushiAiModelCatalog
    public let profileUpdatedAtMs: Int64

    public init(
        preferredModel: String?,
        catalog: FabushiAiModelCatalog,
        profileUpdatedAtMs: Int64
    ) {
        self.selectedPreferredModel = FabushiAiModelCatalog.normalizePreferredModel(preferredModel)
        self.catalog = catalog
        self.profileUpdatedAtMs = profileUpdatedAtMs
    }

    public var isFollowingSettings: Bool {
        selectedPreferredModel.isEmpty
    }

    public var showsMismatchWarning: Bool {
        catalog.isMismatchedOverride(selectedPreferredModel)
    }

    public var showsNoProviderWarning: Bool {
        !catalog.hasConfiguredProvider
    }

    /// Effective displayed model label respecting `updatedAtMs > 0` and live `magpie.state`.
    public var effectiveActiveModelLabel: String? {
        if !selectedPreferredModel.isEmpty {
            return selectedPreferredModel
        }
        if let active = catalog.activeModel, !active.isEmpty {
            if let provider = catalog.activeProvider, !provider.isEmpty {
                return "\(provider) / \(active)"
            }
            return active
        }
        return nil
    }

    public mutating func selectFollowSettings() {
        selectedPreferredModel = ""
    }
}

@MainActor
public struct FabushiInspectorAiModelView: View {
    public let botId: UInt64
    public let initialBotConfig: FabushiBotConfig?
    public let initialComputerProfile: FabushiBotComputerProfile?
    public let initialCatalog: FabushiAiModelCatalog
    public let computers: [FabushiBotComputer]
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore
    public let onSaved: (FabushiBotConfig?, FabushiBotComputerProfile?) -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?

    @State private var catalog: FabushiAiModelCatalog
    @State private var displayName: String
    @State private var descriptionText: String
    @State private var preferredModel: String
    @State private var thinkingLevel: String
    @State private var approvalMode: String
    @State private var personaPrompt: String
    @State private var boundNodeId: String
    @State private var profileUpdatedAtMs: Int64
    @State private var isSaving: Bool = false

    private static let thinkingLevels: [String] = ["off", "minimal", "low", "medium", "high"]
    private static let approvalModes: [String] = ["ask", "auto", "strict"]

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var modelState: FabushiInspectorBotModelState {
        FabushiInspectorBotModelState(
            preferredModel: preferredModel,
            catalog: catalog,
            profileUpdatedAtMs: profileUpdatedAtMs
        )
    }

    public init(
        botId: UInt64,
        initialBotConfig: FabushiBotConfig?,
        initialComputerProfile: FabushiBotComputerProfile?,
        initialCatalog: FabushiAiModelCatalog,
        computers: [FabushiBotComputer],
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore,
        onSaved: @escaping (FabushiBotConfig?, FabushiBotComputerProfile?) -> Void = { _, _ in }
    ) {
        self.botId = botId
        self.initialBotConfig = initialBotConfig
        self.initialComputerProfile = initialComputerProfile
        self.initialCatalog = initialCatalog
        self.computers = computers
        self.coordinator = coordinator
        self.router = router
        self.store = store
        self.onSaved = onSaved

        _catalog = State(initialValue: initialCatalog)
        _displayName = State(initialValue: initialBotConfig?.displayName ?? "")
        _descriptionText = State(initialValue: initialBotConfig?.descriptionText ?? "")

        // `D10-09`: Respect `updatedAtMs > 0` so cleared `""` Follow Settings never resurrects stale bot summary!
        let authoritativeRawModel: String?
        if let profile = initialComputerProfile, profile.updatedAtMs > 0 {
            authoritativeRawModel = profile.preferredModel
        } else {
            authoritativeRawModel = initialBotConfig?.preferredModel ?? initialComputerProfile?.preferredModel
        }
        _preferredModel = State(initialValue: FabushiAiModelCatalog.normalizePreferredModel(authoritativeRawModel))
        _thinkingLevel = State(initialValue: initialBotConfig?.thinkingLevel ?? "medium")
        _approvalMode = State(initialValue: initialComputerProfile?.approvalMode ?? initialBotConfig?.approvalMode ?? "ask")
        _personaPrompt = State(initialValue: initialComputerProfile?.personaPrompt ?? "")
        _boundNodeId = State(initialValue: initialComputerProfile?.boundNodeId ?? "")
        _profileUpdatedAtMs = State(initialValue: max(initialComputerProfile?.updatedAtMs ?? 0, initialBotConfig?.updatedAtMs ?? 0))
    }

    public var body: some View {
        Form {
            // Dynamic AI Model Section (`D10-09`)
            Section {
                if modelState.showsNoProviderWarning {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.warningColor)
                        Text(theme.messagingString(.inspectorNoProviderWarning))
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textSecondary)
                    }
                    .accessibilityIdentifier("fabushi.inspector.aiModel.noProviderWarning")
                }

                if modelState.showsMismatchWarning {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "exclamationmark.circle.fill")
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.warningColor)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(theme.messagingString(.inspectorModelMismatchWarning))
                                .fabushiFont(.footnote)
                                .fontWeight(.semibold)
                                .foregroundStyle(theme.warningColor)
                            Text(preferredModel)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .accessibilityIdentifier("fabushi.inspector.aiModel.mismatchBanner")
                }

                // Follow Settings Pill (`preferred_model == ""`)
                Button {
                    preferredModel = ""
                } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(theme.messagingString(.inspectorFollowSettingsOption))
                                .fabushiFont(.body)
                                .foregroundStyle(theme.textPrimary)
                            if let activeModel = catalog.activeModel, !activeModel.isEmpty {
                                Text(theme.messagingFormat(.inspectorActiveModelCaptionFormat, activeModel))
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                        }
                        Spacer()
                        if modelState.isFollowingSettings {
                            Image(systemName: "checkmark")
                                .fabushiFont(.body)
                                .foregroundStyle(theme.accentColor)
                        }
                    }
                    .contentShape(Rectangle())
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("fabushi.inspector.aiModel.followSettings")

                // Live Models from `magpie.state`
                ForEach(catalog.models) { entry in
                    Button {
                        preferredModel = entry.modelId
                    } label: {
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(entry.displayName)
                                    .fabushiFont(.body)
                                    .foregroundStyle(theme.textPrimary)
                                if !entry.providerId.isEmpty {
                                    Text(entry.providerId)
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.textSecondary)
                                }
                            }
                            Spacer()
                            if preferredModel == entry.modelId {
                                Image(systemName: "checkmark")
                                    .fabushiFont(.body)
                                    .foregroundStyle(theme.accentColor)
                            }
                        }
                        .contentShape(Rectangle())
                        .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }

                // Live Routing Groups from `magpie.state`
                ForEach(catalog.routingGroups, id: \.self) { groupName in
                    Button {
                        preferredModel = groupName
                    } label: {
                        HStack {
                            Label(groupName, systemImage: "arrow.triangle.branch")
                                .fabushiFont(.body)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            if preferredModel == groupName {
                                Image(systemName: "checkmark")
                                    .fabushiFont(.body)
                                    .foregroundStyle(theme.accentColor)
                            }
                        }
                        .contentShape(Rectangle())
                        .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }

                // Model Settings Navigation Shortcut
                Button {
                    router.dismissInspector()
                    router.navigate(to: .settings(section: "aiProviders"), coordinator: coordinator)
                } label: {
                    HStack {
                        Label(theme.messagingString(.inspectorModelSettingsAction), systemImage: "slider.horizontal.3")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                        Spacer()
                        Image(systemName: "chevron.right")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textMuted)
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("fabushi.inspector.aiModel.modelSettingsButton")
            } header: {
                Text(theme.messagingString(.inspectorOverrideModelLabel))
            } footer: {
                if let activeLabel = modelState.effectiveActiveModelLabel {
                    Text(theme.messagingFormat(.inspectorActiveModelCaptionFormat, activeLabel))
                        .fabushiFont(.caption)
                }
            }

            // Bot Identity & Behavior
            Section(header: Text(theme.messagingString(.inspectorBotConfigTitle))) {
                TextField(theme.messagingString(.inspectorChatTitlePlaceholder), text: $displayName)
                    .fabushiFont(.body)

                TextField(theme.messagingString(.inspectorChatAboutPlaceholder), text: $descriptionText, axis: .vertical)
                    .fabushiFont(.body)
                    .lineLimit(2...5)

                Picker(theme.messagingString(.inspectorThinkingLevelLabel), selection: $thinkingLevel) {
                    ForEach(Self.thinkingLevels, id: \.self) { level in
                        Text(level.capitalized).tag(level)
                    }
                }

                Picker(theme.messagingString(.inspectorApprovalModeLabel), selection: $approvalMode) {
                    ForEach(Self.approvalModes, id: \.self) { mode in
                        Text(mode.capitalized).tag(mode)
                    }
                }

                TextField(theme.messagingString(.inspectorPersonaPromptLabel), text: $personaPrompt, axis: .vertical)
                    .fabushiFont(.body)
                    .lineLimit(3...6)
            }

            // Bound Computers (`computers.list`, `computers.botProfileUpdate`)
            Section(header: Text(theme.messagingString(.inspectorBotComputersTitle))) {
                Picker(theme.messagingString(.inspectorBindComputerLabel), selection: $boundNodeId) {
                    Text(theme.messagingString(.inspectorUnboundComputerOption)).tag("")
                    ForEach(computers) { node in
                        Text("\(node.displayName) (\(node.online ? "Online" : "Offline"))")
                            .tag(node.nodeId)
                    }
                }

                if !boundNodeId.isEmpty {
                    Button {
                        router.dismissInspector()
                        router.navigate(to: .remoteComputer(nodeId: boundNodeId), coordinator: coordinator)
                    } label: {
                        Label(theme.messagingString(.computerCardOpenAction), systemImage: "desktopcomputer")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .navigationTitle(theme.messagingString(.inspectorBotConfigTitle))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button(theme.messagingString(.inspectorSaveChangesAction)) {
                    Task { @MainActor in
                        await saveBotConfiguration()
                    }
                }
                .disabled(isSaving)
            }
        }
        .task {
            let refreshed = await store.client.fetchEffectiveAiModelCatalog()
            if refreshed.hasConfiguredProvider || !refreshed.routingGroups.isEmpty {
                catalog = refreshed
            }
        }
    }

    private func saveBotConfiguration() async {
        guard !isSaving else { return }
        isSaving = true
        defer { isSaving = false }

        let normalizedModel = FabushiAiModelCatalog.normalizePreferredModel(preferredModel)
        let shouldClearNode = boundNodeId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty

        async let patchedBotTask = store.client.patchBot(
            botId: botId,
            displayName: displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : displayName,
            descriptionText: descriptionText,
            preferredModel: normalizedModel,
            thinkingLevel: thinkingLevel,
            approvalMode: approvalMode
        )
        async let updatedProfileTask = store.client.updateBotComputerProfile(
            botId: botId,
            boundNodeId: shouldClearNode ? nil : boundNodeId,
            clearBoundNode: shouldClearNode,
            personaPrompt: personaPrompt,
            preferredModel: normalizedModel,
            approvalMode: approvalMode
        )

        let patchedBot = try? await patchedBotTask
        let updatedProfile = try? await updatedProfileTask

        guard patchedBot != nil || updatedProfile != nil else {
            router.toastController.showNonChatToast(
                theme.messagingString(.inspectorSaveFailedToast),
                isError: true
            )
            return
        }

        profileUpdatedAtMs = max(updatedProfile?.updatedAtMs ?? 0, FabushiWireDateParser.nowMs())
        onSaved(patchedBot, updatedProfile)
        router.toastController.showNonChatToast(
            theme.messagingString(.inspectorSavedChangesToast),
            isError: false
        )
    }
}
