import Foundation
import SwiftUI

/// Full Apple HIG Magpie AI Provider Gateway settings view (`FabushiAccessibilityIDs.Magpie.root`).
///
/// Pushed onto `FabushiSettingsRootView`'s `NavigationStack` (strictly ZERO nested root `NavigationStack`
/// and ZERO conflicting `.topBarLeading` toolbar buttons; modal sheets wrap in their own `NavigationStack`).
@MainActor
public struct FabushiMagpieSettingsView: View {
    @State private var viewModel: FabushiMagpieViewModel
    @State private var presetSearchQuery: String = ""
    @State private var selectedPresetKind: FabushiMagpiePresetKind?
    @State private var isAddProviderSheetPresented: Bool = false
    @State private var isAddGroupSheetPresented: Bool = false
    @State private var routeTestTarget: String = ""
    @State private var customWordsInput: String = ""
    @State private var oauthAuthorizeUrlDraft: String = ""

    // Add Provider BYOK sheet fields
    @State private var draftProviderId: String = ""
    @State private var draftProviderName: String = ""
    @State private var draftProtocol: FabushiMagpieProtocolKind = .chatCompletions
    @State private var draftBaseUrl: String = ""
    @State private var draftDefaultModel: String = ""
    @State private var draftApiKey: String = ""
    @State private var draftPresetId: String?

    // Add Routing Group sheet fields
    @State private var draftGroupId: String = ""
    @State private var draftGroupName: String = ""
    @State private var draftGroupStrategy: FabushiMagpieRoutingStrategy = .smart
    @State private var draftGroupMembersText: String = ""

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        cleanupHub: FabushiAgentStudioLogoutCleanupHub = FabushiAgentStudioLogoutCleanupHub()
    ) {
        self._viewModel = State(
            initialValue: FabushiMagpieViewModel(
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

        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let classified = viewModel.lastClassifiedError {
                    FabushiStudioClassifiedErrorBanner(
                        error: classified,
                        onOpenAiProviders: {
                            isAddProviderSheetPresented = true
                        },
                        onDismiss: {
                            viewModel.dismissClassifiedError()
                        }
                    )
                }

                noDotfileScanNoticeBanner(locale: locale)
                activeSubscriptionsSection(locale: locale, reduceMotion: reduceMotion)
                configuredProvidersAndProbeSection(locale: locale)
                presetsCatalogSection(locale: locale, isRegularWidth: isRegularWidth)
                smartRoutingSection(locale: locale)
                piiMaskingSection(locale: locale)
                importSection(locale: locale)
                usageMetricsSection(locale: locale, isRegularWidth: isRegularWidth)
                vaultAndBrowserOAuthSection(locale: locale)
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiAgentStudioStrings.text("m4.magpie.title", locale: locale))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { @MainActor in
                        await viewModel.loadAll()
                    }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .fabushiMinimumTouchTarget()
                .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.refresh", locale: locale))
                .accessibilityIdentifier("fabushi.internal.magpie.refreshButton")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Magpie.root)
        .task(id: viewModel.coordinator.accountScopeGeneration) {
            await viewModel.loadAll()
            customWordsInput = viewModel.maskingConfig.customWords.joined(separator: ", ")
            if routeTestTarget.isEmpty, let first = viewModel.routingGroups.first {
                routeTestTarget = first.qualifiedModelName
            }
        }
        .onChange(of: viewModel.coordinator.accountScopeGeneration) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .onChange(of: viewModel.coordinator.activeAccountId) { _, _ in
            viewModel.syncAccountScopeIfNeeded()
        }
        .sheet(isPresented: $isAddProviderSheetPresented) {
            addProviderSheet(locale: locale)
                .environment(theme)
        }
        .sheet(isPresented: $isAddGroupSheetPresented) {
            addRoutingGroupSheet(locale: locale)
                .environment(theme)
        }
    }

    // MARK: - `IOS-MAIN-INV-04` No-Dotfile-Scan Notice

    @ViewBuilder
    private func noDotfileScanNoticeBanner(locale: FabushiLocale) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "lock.shield.fill")
                .foregroundStyle(theme.accentColor)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(FabushiAgentStudioStrings.text("m4.common.iosSandboxNotice", locale: locale))
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                Text(viewModel.lastSandboxNotice ?? FabushiAgentStudioStrings.text("m4.magpie.noDotfileScanNotice", locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("fabushi.internal.magpie.sandboxNotice")
    }

    // MARK: - Active Subscriptions & `D10-10` Single-Tap Instant Model Switch

    @ViewBuilder
    private func activeSubscriptionsSection(locale: FabushiLocale, reduceMotion: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.subscriptionsSection", locale: locale))

            if viewModel.subscriptions.isEmpty {
                Text(FabushiAgentStudioStrings.text("m4.magpie.emptySubscriptions", locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(
                        theme.surfaceSecondary,
                        in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                    )
            } else {
                ForEach(viewModel.subscriptions) { sub in
                    subscriptionCard(sub: sub, locale: locale, reduceMotion: reduceMotion)
                }
            }
        }
    }

    @ViewBuilder
    private func subscriptionCard(
        sub: FabushiMagpieSubscriptionDTO,
        locale: FabushiLocale,
        reduceMotion: Bool
    ) -> some View {
        let isActive = sub.isActiveFor(activeProvider: viewModel.activeProvider)
        let models = sub.candidateModels(fallbackActiveModel: isActive ? viewModel.activeModel : "")
        let selectedModel = viewModel.selectedSubscriptionModels[sub.id] ?? sub.defaultModel ?? ""
        let borderColor = isActive ? theme.accentColor : theme.hairlineBorder
        let borderWidth: CGFloat = isActive ? 1.25 : 0.75

        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    subscriptionIdentity(sub: sub, isActive: isActive, locale: locale)
                    Spacer(minLength: 8)
                    if !isActive {
                        subscriptionActivateButton(subId: sub.id, locale: locale)
                    }
                }
                VStack(alignment: .leading, spacing: 8) {
                    subscriptionIdentity(sub: sub, isActive: isActive, locale: locale)
                    if !isActive {
                        subscriptionActivateButton(subId: sub.id, locale: locale)
                    }
                }
            }

            if !models.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(models, id: \.self) { model in
                            subscriptionModelChip(
                                subId: sub.id,
                                model: model,
                                isSelected: selectedModel == model,
                                reduceMotion: reduceMotion
                            )
                        }
                    }
                }
            }
        }
        .padding(12)
        .background(
            theme.surfaceSecondary,
            in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(borderColor, lineWidth: borderWidth)
        )
    }

    @ViewBuilder
    private func subscriptionActivateButton(subId: String, locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.magpie.activateSubscription", locale: locale),
            systemImage: "bolt.fill",
            accessibilityID: "fabushi.internal.magpie.activate.\(subId)",
            isProminent: true
        ) {
            viewModel.activateSubscription(subscriptionId: subId)
        }
    }

    @ViewBuilder
    private func subscriptionModelChip(
        subId: String,
        model: String,
        isSelected: Bool,
        reduceMotion: Bool
    ) -> some View {
        let fgColor = isSelected ? theme.accentForegroundColor : theme.textPrimary
        let bgColor = isSelected ? theme.accentColor : theme.surfaceElevated
        let weight: Font.Weight = isSelected ? .semibold : .regular

        Button {
            if reduceMotion {
                _ = viewModel.selectSubscriptionModel(subscriptionId: subId, model: model)
            } else {
                withAnimation(.easeInOut(duration: 0.16)) {
                    _ = viewModel.selectSubscriptionModel(subscriptionId: subId, model: model)
                }
            }
        } label: {
            Text(model)
                .fabushiFont(.caption)
                .fontWeight(weight)
                .foregroundStyle(fgColor)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .fabushiMinimumTouchTarget()
                .background(bgColor, in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? [.isSelected] : [])
        .accessibilityIdentifier("fabushi.internal.magpie.subModel.\(subId).\(model)")
    }

    @ViewBuilder
    private func subscriptionIdentity(
        sub: FabushiMagpieSubscriptionDTO,
        isActive: Bool,
        locale: FabushiLocale
    ) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(sub.label)
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                if isActive {
                    Text(FabushiAgentStudioStrings.text("m4.magpie.activeBadge", locale: locale))
                        .fabushiFont(.caption2)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.accentForegroundColor)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(theme.accentColor, in: Capsule())
                }
            }
            Text([sub.kind, sub.email, sub.plan].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
        }
    }

    // MARK: - Configured Providers List & Health Probe

    @ViewBuilder
    private func configuredProvidersAndProbeSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.providersSection", locale: locale))
                    Spacer(minLength: 8)
                    addProviderActionButton(locale: locale)
                }
                VStack(alignment: .leading, spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.providersSection", locale: locale))
                    addProviderActionButton(locale: locale)
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                if viewModel.providers.isEmpty {
                    Text(FabushiAgentStudioStrings.text("m4.magpie.emptyProviders", locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                        .padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    ForEach(viewModel.providers) { provider in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 8) {
                                Text(provider.name)
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.textPrimary)
                                Spacer(minLength: 4)
                                Text(provider.apiMode)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.textSecondary)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 3)
                                    .background(theme.surfaceElevated, in: Capsule())
                            }
                            if let baseUrl = provider.baseUrl, !baseUrl.isEmpty {
                                Text(baseUrl)
                                    .fabushiFont(.monospacedCaption)
                                    .foregroundStyle(theme.textSecondary)
                                    .lineLimit(2)
                            }
                            HStack(spacing: 10) {
                                if !provider.defaultModel.isEmpty {
                                    Text(provider.defaultModel)
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.accentColor)
                                }
                                if let masked = provider.maskedKey, !masked.isEmpty {
                                    Text(masked)
                                        .fabushiFont(.monospacedCaption)
                                        .foregroundStyle(theme.textMuted)
                                }
                            }
                        }
                        .padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            }
            .padding(10)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Magpie.providerList)

            // Health Probe Row (`probeHealthButton` + `probeStatusLabel`)
            VStack(alignment: .leading, spacing: 8) {
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.magpie.probeHealth", locale: locale),
                    systemImage: "waveform.path.ecg",
                    accessibilityID: FabushiAccessibilityIDs.Magpie.probeHealthButton,
                    isProminent: false
                ) {
                    Task { @MainActor in
                        await viewModel.probeProviderHealth(locale: locale)
                    }
                }
                .disabled(viewModel.isProbing)

                Text(
                    viewModel.probeStatusText.isEmpty
                        ? FabushiAgentStudioStrings.text("m4.magpie.probePendingM6Notice", locale: locale)
                        : viewModel.probeStatusText
                )
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Magpie.probeStatusLabel)
            }
            .padding(12)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        }
    }

    @ViewBuilder
    private func addProviderActionButton(locale: FabushiLocale) -> some View {
        FabushiGlassPillButton(
            title: FabushiAgentStudioStrings.text("m4.magpie.addProvider", locale: locale),
            systemImage: "plus.circle.fill",
            accessibilityID: FabushiAccessibilityIDs.Magpie.addProviderButton,
            isProminent: true
        ) {
            draftPresetId = nil
            draftProviderId = ""
            draftProviderName = ""
            draftProtocol = .chatCompletions
            draftBaseUrl = ""
            draftDefaultModel = ""
            draftApiKey = ""
            isAddProviderSheetPresented = true
        }
    }

    // MARK: - Built-in Multi-Provider Presets & `models.dev` Catalog Filter

    @ViewBuilder
    private func presetsCatalogSection(locale: FabushiLocale, isRegularWidth: Bool) -> some View {
        let filteredPresets = viewModel.presets.filter { preset in
            if let kind = selectedPresetKind, preset.kind != kind {
                return false
            }
            let query = presetSearchQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if query.isEmpty { return true }
            return preset.name.lowercased().contains(query)
                || preset.id.lowercased().contains(query)
                || preset.defaultModel.lowercased().contains(query)
                || (preset.catalog?.lowercased().contains(query) ?? false)
                || preset.suggestedModels.contains(where: { $0.lowercased().contains(query) })
        }

        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.presetsSection", locale: locale))

            TextField(
                FabushiAgentStudioStrings.text("m4.magpie.presetSearchPlaceholder", locale: locale),
                text: $presetSearchQuery
            )
            .fabushiFont(.body)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .padding(10)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityIdentifier("fabushi.internal.magpie.presetSearchField")

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    presetKindPill(title: FabushiAgentStudioStrings.text("m4.mcp.filterAll", locale: locale), kind: nil)
                    ForEach(FabushiMagpiePresetKind.allCases) { kind in
                        presetKindPill(title: kind.rawValue.capitalized, kind: kind)
                    }
                }
            }

            let columns = isRegularWidth
                ? [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)]
                : [GridItem(.flexible())]

            LazyVGrid(columns: columns, spacing: 8) {
                ForEach(filteredPresets.prefix(12)) { preset in
                    Button {
                        Task { @MainActor in
                            let resolved = await viewModel.resolvePresetEndpoints(presetId: preset.id)
                            draftPresetId = preset.id
                            draftProviderId = preset.id
                            draftProviderName = preset.name
                            draftProtocol = resolved?.protocolKind ?? preset.defaultProtocol
                            draftBaseUrl = resolved?.baseUrl ?? preset.chatUrl ?? preset.anthropicUrl ?? preset.geminiUrl ?? ""
                            draftDefaultModel = resolved?.defaultModel ?? preset.defaultModel
                            draftApiKey = ""
                            isAddProviderSheetPresented = true
                        }
                    } label: {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 6) {
                                Text(preset.name)
                                    .fabushiFont(.subheadline)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(theme.textPrimary)
                                Spacer(minLength: 4)
                                Text(preset.kind.rawValue)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.textSecondary)
                            }
                            if !preset.defaultModel.isEmpty {
                                Text(preset.defaultModel)
                                    .fabushiFont(.monospacedCaption)
                                    .foregroundStyle(theme.accentColor)
                                    .lineLimit(1)
                            }
                            if let catalog = preset.catalog, !catalog.isEmpty {
                                Text("models.dev · \(catalog)")
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.textMuted)
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fabushiMinimumTouchTarget()
                        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("fabushi.internal.magpie.preset.\(preset.id)")
                }
            }
        }
    }

    @ViewBuilder
    private func presetKindPill(title: String, kind: FabushiMagpiePresetKind?) -> some View {
        let selected = selectedPresetKind == kind
        Button {
            selectedPresetKind = kind
        } label: {
            Text(title)
                .fabushiFont(.caption)
                .fontWeight(selected ? .semibold : .regular)
                .foregroundStyle(selected ? theme.accentForegroundColor : theme.textPrimary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .fabushiMinimumTouchTarget()
                .background(selected ? theme.accentColor : theme.surfaceSecondary, in: Capsule())
        }
        .buttonStyle(.plain)
    }

    // MARK: - Smart Routing Groups & Circuit Breaker Badge

    @ViewBuilder
    private func smartRoutingSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.routingSection", locale: locale))
                    Spacer(minLength: 8)
                    circuitBreakerBadge(locale: locale)
                }
                VStack(alignment: .leading, spacing: 6) {
                    FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.routingSection", locale: locale))
                    circuitBreakerBadge(locale: locale)
                }
            }

            VStack(alignment: .leading, spacing: 10) {
                Toggle(
                    FabushiAgentStudioStrings.text("m4.magpie.routingToggle", locale: locale),
                    isOn: $viewModel.smartRoutingEnabled
                )
                .fabushiFont(.body)
                .tint(theme.accentColor)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Magpie.routingRuleToggle)

                ForEach(viewModel.routingGroups) { group in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(spacing: 8) {
                            Text(group.name)
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Text(group.qualifiedModelName)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.accentColor)
                            Spacer(minLength: 4)
                            Text(group.strategy.displayName)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(theme.surfaceElevated, in: Capsule())
                            Button(role: .destructive) {
                                Task { @MainActor in
                                    _ = await viewModel.deleteRoutingGroup(id: group.canonicalId)
                                }
                            } label: {
                                Image(systemName: "trash")
                                    .foregroundStyle(theme.destructiveColor)
                            }
                            .buttonStyle(.plain)
                            .fabushiMinimumTouchTarget()
                            .accessibilityLabel(FabushiAgentStudioStrings.text("m4.common.delete", locale: locale))
                        }

                        if !group.members.isEmpty {
                            Text(group.members.map(\.target).joined(separator: " · "))
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .padding(10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) {
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.magpie.addGroup", locale: locale),
                            systemImage: "plus",
                            accessibilityID: "fabushi.internal.magpie.addGroupButton",
                            isProminent: false
                        ) {
                            draftGroupId = ""
                            draftGroupName = ""
                            draftGroupStrategy = .smart
                            draftGroupMembersText = ""
                            isAddGroupSheetPresented = true
                        }

                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.magpie.resolveRoute", locale: locale),
                            systemImage: "arrow.triangle.branch",
                            accessibilityID: "fabushi.internal.magpie.resolveRouteButton",
                            isProminent: false
                        ) {
                            Task { @MainActor in
                                let target = routeTestTarget.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                                    ? (viewModel.routingGroups.first?.qualifiedModelName ?? "deepseek/deepseek-chat")
                                    : routeTestTarget
                                _ = await viewModel.resolveRoute(target: target)
                            }
                        }
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.magpie.addGroup", locale: locale),
                            systemImage: "plus",
                            accessibilityID: "fabushi.internal.magpie.addGroupButton",
                            isProminent: false
                        ) {
                            draftGroupId = ""
                            draftGroupName = ""
                            draftGroupStrategy = .smart
                            draftGroupMembersText = ""
                            isAddGroupSheetPresented = true
                        }

                        FabushiGlassPillButton(
                            title: FabushiAgentStudioStrings.text("m4.magpie.resolveRoute", locale: locale),
                            systemImage: "arrow.triangle.branch",
                            accessibilityID: "fabushi.internal.magpie.resolveRouteButton",
                            isProminent: false
                        ) {
                            Task { @MainActor in
                                let target = routeTestTarget.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                                    ? (viewModel.routingGroups.first?.qualifiedModelName ?? "deepseek/deepseek-chat")
                                    : routeTestTarget
                                _ = await viewModel.resolveRoute(target: target)
                            }
                        }
                    }
                }

                TextField(
                    FabushiAgentStudioStrings.text("m4.magpie.routeTargetPlaceholder", locale: locale),
                    text: $routeTestTarget
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                if !viewModel.resolvedRoutes.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(viewModel.resolvedRoutes) { route in
                            Text("\(route.providerId)/\(route.model) · \(route.strategy.rawValue) (\(route.reason))")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                }
            }
            .padding(12)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }

    @ViewBuilder
    private func circuitBreakerBadge(locale: FabushiLocale) -> some View {
        let badgeColor: Color = {
            switch viewModel.circuitBreakerState {
            case .closed: return theme.successColor
            case .open: return theme.destructiveColor
            case .halfOpen: return theme.warningColor
            }
        }()

        HStack(spacing: 5) {
            Circle()
                .fill(badgeColor)
                .frame(width: 8, height: 8)
                .accessibilityHidden(true)
            Text(FabushiAgentStudioStrings.text(viewModel.circuitBreakerState.localizationKey, locale: locale))
                .fabushiFont(.caption2)
                .fontWeight(.semibold)
                .foregroundStyle(theme.textPrimary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(badgeColor.opacity(0.16), in: Capsule())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Magpie.circuitBreakerBadge)
    }

    // MARK: - PII Privacy Masking & Live Preview

    @ViewBuilder
    private func piiMaskingSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.piiSection", locale: locale))

            VStack(alignment: .leading, spacing: 10) {
                Toggle(
                    FabushiAgentStudioStrings.text("m4.magpie.piiToggle", locale: locale),
                    isOn: Binding(
                        get: { viewModel.maskingConfig.enabled },
                        set: { newValue in
                            var updated = viewModel.maskingConfig
                            updated.enabled = newValue
                            viewModel.maskingConfig = updated
                            Task { @MainActor in
                                _ = await viewModel.updateMaskingConfig(updated)
                            }
                        }
                    )
                )
                .fabushiFont(.body)
                .tint(theme.accentColor)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Magpie.piiMaskingToggle)

                Toggle(
                    FabushiAgentStudioStrings.text("m4.magpie.piiSecretsToggle", locale: locale),
                    isOn: Binding(
                        get: { viewModel.maskingConfig.maskSecrets },
                        set: { newValue in
                            var updated = viewModel.maskingConfig
                            updated.maskSecrets = newValue
                            viewModel.maskingConfig = updated
                            Task { @MainActor in
                                _ = await viewModel.updateMaskingConfig(updated)
                            }
                        }
                    )
                )
                .fabushiFont(.subheadline)
                .tint(theme.accentColor)

                Toggle(
                    FabushiAgentStudioStrings.text("m4.magpie.piiPersonalToggle", locale: locale),
                    isOn: Binding(
                        get: { viewModel.maskingConfig.maskPersonal },
                        set: { newValue in
                            var updated = viewModel.maskingConfig
                            updated.maskPersonal = newValue
                            viewModel.maskingConfig = updated
                            Task { @MainActor in
                                _ = await viewModel.updateMaskingConfig(updated)
                            }
                        }
                    )
                )
                .fabushiFont(.subheadline)
                .tint(theme.accentColor)

                TextField(
                    FabushiAgentStudioStrings.text("m4.magpie.customWordsPlaceholder", locale: locale),
                    text: $customWordsInput
                )
                .fabushiFont(.caption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .onSubmit {
                    var updated = viewModel.maskingConfig
                    updated.customWords = customWordsInput
                        .split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                        .filter { !$0.isEmpty }
                    Task { @MainActor in
                        _ = await viewModel.updateMaskingConfig(updated)
                    }
                }

                TextField(
                    FabushiAgentStudioStrings.text("m4.magpie.piiPreviewPlaceholder", locale: locale),
                    text: $viewModel.maskingPreviewInput,
                    axis: .vertical
                )
                .fabushiFont(.body)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(2...4)
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityIdentifier("fabushi.internal.magpie.piiPreviewInput")

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.magpie.piiPreviewButton", locale: locale),
                    systemImage: "eye.slash.fill",
                    accessibilityID: "fabushi.internal.magpie.piiPreviewButton",
                    isProminent: false
                ) {
                    Task { @MainActor in
                        _ = await viewModel.previewMasking(text: viewModel.maskingPreviewInput)
                    }
                }
                .disabled(viewModel.maskingPreviewInput.isEmpty)

                if let preview = viewModel.maskingPreviewResult {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(preview.maskedText)
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textPrimary)
                        if !preview.hits.isEmpty {
                            Text(preview.hits.map { "\($0.kind): \($0.previewHint)" }.joined(separator: " · "))
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
            .padding(12)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }

    // MARK: - `.env` & Deep-Link Import

    @ViewBuilder
    private func importSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.importSection", locale: locale))

            VStack(alignment: .leading, spacing: 10) {
                TextField(
                    FabushiAgentStudioStrings.text("m4.magpie.importLinkPlaceholder", locale: locale),
                    text: $viewModel.importLinkDraft
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                TextField(
                    FabushiAgentStudioStrings.text("m4.magpie.importDotenvPlaceholder", locale: locale),
                    text: $viewModel.importDotenvDraft,
                    axis: .vertical
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .lineLimit(2...5)
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.magpie.importButton", locale: locale),
                    systemImage: "square.and.arrow.down",
                    accessibilityID: "fabushi.internal.magpie.importButton",
                    isProminent: false
                ) {
                    Task { @MainActor in
                        if !viewModel.importLinkDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            _ = await viewModel.importFromLink(viewModel.importLinkDraft)
                        }
                        if !viewModel.importDotenvDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            _ = await viewModel.importFromDotenv(viewModel.importDotenvDraft)
                        }
                    }
                }
                .disabled(
                    viewModel.importLinkDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        && viewModel.importDotenvDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                )
            }
            .padding(12)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }

    // MARK: - Usage & Cost Metrics

    @ViewBuilder
    private func usageMetricsSection(locale: FabushiLocale, isRegularWidth: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.usageSection", locale: locale))

            if viewModel.usageSummary.totalRecords == 0 {
                Text(FabushiAgentStudioStrings.text("m4.magpie.usageEmpty", locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            } else {
                let columns = isRegularWidth
                    ? [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)]
                    : [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)]

                LazyVGrid(columns: columns, spacing: 10) {
                    usageTile(title: "Records", value: "\(viewModel.usageSummary.totalRecords)")
                    usageTile(title: "Tokens", value: "\(viewModel.usageSummary.totalTokens)")
                    usageTile(title: "Cost (USD)", value: String(format: "$%.4f", viewModel.usageSummary.totalCostUsd))
                }
            }
        }
    }

    @ViewBuilder
    private func usageTile(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
            Text(value)
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    // MARK: - Zero-Knowledge Vault AI OAuth Sync & Browser OAuth

    @ViewBuilder
    private func vaultAndBrowserOAuthSection(locale: FabushiLocale) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            FabushiSectionHeader(FabushiAgentStudioStrings.text("m4.magpie.vaultOAuthSync", locale: locale))

            VStack(alignment: .leading, spacing: 10) {
                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.magpie.vaultOAuthSync", locale: locale),
                    systemImage: "key.icloud.fill",
                    accessibilityID: FabushiAccessibilityIDs.Magpie.vaultOAuthSyncButton,
                    isProminent: true
                ) {
                    Task { @MainActor in
                        _ = await viewModel.syncVaultAiOAuth(locale: locale)
                    }
                }

                if let status = viewModel.oauthStatusMessage, !status.isEmpty {
                    Text(status)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }

                ForEach(viewModel.vaultOAuthRecords) { record in
                    HStack(spacing: 8) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(record.providerName)
                                .fabushiFont(.subheadline)
                                .fontWeight(.semibold)
                                .foregroundStyle(theme.textPrimary)
                            Text("\(record.itemKey) · rev \(record.revision)")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                        Spacer(minLength: 4)
                        if !record.maskedTokenPreview.isEmpty {
                            Text(record.maskedTokenPreview)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textMuted)
                        }
                    }
                    .padding(10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }

                Divider().overlay(theme.separatorHairline)

                TextField(
                    FabushiAgentStudioStrings.text("m4.magpie.oauthUrlPlaceholder", locale: locale),
                    text: $oauthAuthorizeUrlDraft
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                FabushiGlassPillButton(
                    title: FabushiAgentStudioStrings.text("m4.magpie.webOAuthButton", locale: locale),
                    systemImage: "safari.fill",
                    accessibilityID: "fabushi.internal.magpie.webOAuthButton",
                    isProminent: false
                ) {
                    viewModel.startBrowserOAuth(authorizeURLString: oauthAuthorizeUrlDraft)
                }
                .disabled(oauthAuthorizeUrlDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(12)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        }
    }

    // MARK: - Modal Sheets (Wrapped in `NavigationStack`)

    @ViewBuilder
    private func addProviderSheet(locale: FabushiLocale) -> some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.magpie.providerNamePlaceholder", locale: locale),
                        text: $draftProviderName
                    )
                    TextField("Provider ID", text: $draftProviderId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Picker("Protocol", selection: $draftProtocol) {
                        ForEach(FabushiMagpieProtocolKind.allCases) { proto in
                            Text(proto.displayName).tag(proto)
                        }
                    }
                    TextField(
                        FabushiAgentStudioStrings.text("m4.magpie.baseUrlPlaceholder", locale: locale),
                        text: $draftBaseUrl
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    TextField(
                        FabushiAgentStudioStrings.text("m4.magpie.defaultModelPlaceholder", locale: locale),
                        text: $draftDefaultModel
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    SecureField(
                        FabushiAgentStudioStrings.text("m4.magpie.apiKeyPlaceholder", locale: locale),
                        text: $draftApiKey
                    )
                }
            }
            .navigationTitle(FabushiAgentStudioStrings.text("m4.magpie.addProvider", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                        isAddProviderSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(FabushiAgentStudioStrings.text("m4.common.save", locale: locale)) {
                        let resolvedId = draftProviderId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? draftProviderName.lowercased().replacingOccurrences(of: " ", with: "-")
                            : draftProviderId
                        Task { @MainActor in
                            _ = await viewModel.saveProviderSpec(
                                providerId: resolvedId,
                                name: draftProviderName,
                                protocolKind: draftProtocol,
                                baseUrl: draftBaseUrl,
                                defaultModel: draftDefaultModel,
                                apiKeyOrToken: draftApiKey,
                                presetId: draftPresetId,
                                locale: locale
                            )
                            isAddProviderSheetPresented = false
                        }
                    }
                    .disabled(draftProviderName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }

    @ViewBuilder
    private func addRoutingGroupSheet(locale: FabushiLocale) -> some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiAgentStudioStrings.text("m4.magpie.groupIdPlaceholder", locale: locale),
                        text: $draftGroupId
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()

                    TextField(
                        FabushiAgentStudioStrings.text("m4.magpie.groupNamePlaceholder", locale: locale),
                        text: $draftGroupName
                    )

                    Picker(
                        FabushiAgentStudioStrings.text("m4.magpie.strategyLabel", locale: locale),
                        selection: $draftGroupStrategy
                    ) {
                        ForEach(FabushiMagpieRoutingStrategy.allCases) { strat in
                            Text(strat.displayName).tag(strat)
                        }
                    }

                    TextField(
                        FabushiAgentStudioStrings.text("m4.magpie.groupMembersPlaceholder", locale: locale),
                        text: $draftGroupMembersText,
                        axis: .vertical
                    )
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                }
            }
            .navigationTitle(FabushiAgentStudioStrings.text("m4.magpie.addGroup", locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(FabushiAgentStudioStrings.text("m4.common.cancel", locale: locale)) {
                        isAddGroupSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(FabushiAgentStudioStrings.text("m4.common.save", locale: locale)) {
                        let members = draftGroupMembersText
                            .split(separator: ",")
                            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                            .filter { !$0.isEmpty }
                            .map { FabushiMagpieRoutingGroupMemberDTO(target: $0) }
                        let group = FabushiMagpieRoutingGroupDTO(
                            id: draftGroupId,
                            name: draftGroupName.isEmpty ? draftGroupId : draftGroupName,
                            strategy: draftGroupStrategy,
                            members: members
                        )
                        Task { @MainActor in
                            _ = await viewModel.upsertRoutingGroup(group)
                            isAddGroupSheetPresented = false
                        }
                    }
                    .disabled(draftGroupId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
