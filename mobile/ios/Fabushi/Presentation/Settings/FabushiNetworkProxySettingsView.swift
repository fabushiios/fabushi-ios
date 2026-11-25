import SwiftUI

/// Native Apple HIG Network, Server URL & Strict Proxy (`D10-13`) Settings view.
@MainActor
public struct FabushiNetworkProxySettingsView: View {
    @State private var viewModel: FabushiNetworkProxyViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        _viewModel = State(
            initialValue: FabushiNetworkProxyViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        List {
            // MARK: Editable Server URL (`coordinator.setServerURL`)
            Section(FabushiPlatformStrings.text(.settingsServerUrlSectionTitle, locale: locale)) {
                TextField(
                    FabushiPlatformStrings.text(.settingsServerUrlPlaceholder, locale: locale),
                    text: $viewModel.serverUrlDraft
                )
                .fabushiFont(.body)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()

                Button {
                    Task { _ = await viewModel.applyServerURL(locale: locale) }
                } label: {
                    Text(FabushiPlatformStrings.text(.settingsServerUrlApplyAction, locale: locale))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)

                Text(viewModel.activeServerUrl)
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textSecondary)
            }

            // MARK: Proxy Editor & Live Latency Probe (`D10-13`)
            Section(FabushiPlatformStrings.text(.settingsProxySectionTitle, locale: locale)) {
                Picker(
                    FabushiPlatformStrings.text(.settingsProxySectionTitle, locale: locale),
                    selection: $viewModel.draftKind
                ) {
                    ForEach(FabushiProxyKind.allCases) { kind in
                        Text(kind.localizedTitle(locale: locale)).tag(kind)
                    }
                }
                .pickerStyle(.segmented)

                if viewModel.draftKind != .direct {
                    TextField(
                        FabushiPlatformStrings.text(.settingsProxyHostPlaceholder, locale: locale),
                        text: $viewModel.draftHost
                    )
                    .fabushiFont(.body)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.proxyHostField)

                    TextField(
                        FabushiPlatformStrings.text(.settingsProxyPortPlaceholder, locale: locale),
                        text: $viewModel.draftPort
                    )
                    .fabushiFont(.body)
                    .keyboardType(.numberPad)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.proxyPortField)

                    if viewModel.draftKind == .mtproto {
                        TextField(
                            FabushiPlatformStrings.text(.settingsProxyMtprotoSecretPlaceholder, locale: locale),
                            text: $viewModel.draftMtprotoSecret
                        )
                        .fabushiFont(.monospacedCaption)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    } else {
                        TextField(
                            FabushiPlatformStrings.text(.settingsProxyUsernamePlaceholder, locale: locale),
                            text: $viewModel.draftUsername
                        )
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()

                        SecureField(
                            FabushiPlatformStrings.text(.settingsProxyPasswordPlaceholder, locale: locale),
                            text: $viewModel.draftPassword
                        )
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    }
                }

                HStack(spacing: 12) {
                    Button {
                        Task { _ = await viewModel.saveDraftProxy(activate: true, locale: locale) }
                    } label: {
                        Text(FabushiPlatformStrings.text(.settingsProxyAddAction, locale: locale))
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.accentForegroundColor)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 8)
                            .background(theme.accentColor, in: Capsule())
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)

                    Button {
                        Task { _ = await viewModel.probeDraftProxy(locale: locale) }
                    } label: {
                        HStack(spacing: 6) {
                            if viewModel.isProbing {
                                ProgressView()
                            }
                            Text(FabushiPlatformStrings.text(.settingsProxyProbeAction, locale: locale))
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.accentColor)
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(theme.surfaceElevated, in: Capsule())
                        .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .disabled(viewModel.isProbing)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.proxyProbeButton)
                }

                Text(
                    viewModel.probeStatusText.isEmpty
                        ? FabushiPlatformStrings.text(.settingsProxyStatusDirect, locale: locale)
                        : viewModel.probeStatusText
                )
                .fabushiFont(.caption)
                .foregroundStyle(viewModel.lastProbeLatencyMs != nil ? theme.successColor : theme.textSecondary)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.proxyStatusLabel)
            }

            // MARK: Saved Proxy Profiles & RFC-3986 Share Links
            Section {
                Button {
                    Task { _ = await viewModel.selectActiveProxy(id: nil, locale: locale) }
                } label: {
                    HStack {
                        Label(
                            FabushiPlatformStrings.text(.settingsProxyDisableAction, locale: locale),
                            systemImage: "bolt.horizontal"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                        Spacer()
                        if viewModel.activeProxyId == nil {
                            Image(systemName: "checkmark")
                                .foregroundStyle(theme.accentColor)
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)

                ForEach(viewModel.proxies) { proxy in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Button {
                                guard proxy.kind.isRustHostSupported else { return }
                                Task { _ = await viewModel.selectActiveProxy(id: proxy.id, locale: locale) }
                            } label: {
                                HStack(spacing: 8) {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text("\(proxy.kind.localizedTitle(locale: locale)) · \(proxy.displayEndpoint)")
                                            .fabushiFont(.subheadline)
                                            .foregroundStyle(theme.textPrimary)
                                        if let latency = proxy.lastLatencyMs {
                                            Text(FabushiPlatformStrings.proxyLatencyFormatted(latency, locale: locale))
                                                .fabushiFont(.caption2)
                                                .foregroundStyle(theme.successColor)
                                        }
                                    }
                                    Spacer()
                                    if viewModel.activeProxyId == proxy.id {
                                        Image(systemName: "checkmark.circle.fill")
                                            .foregroundStyle(theme.accentColor)
                                    }
                                }
                                .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                        }

                        HStack(spacing: 12) {
                            if !FabushiProxyDeepLinks.exportLink(for: proxy).isEmpty {
                                Button {
                                    _ = viewModel.copyShareLink(for: proxy, locale: locale)
                                } label: {
                                    Label(
                                        FabushiPlatformStrings.text(.settingsProxyShareLinkAction, locale: locale),
                                        systemImage: "link"
                                    )
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.accentColor)
                                    .fabushiMinimumTouchTarget()
                                }
                                .buttonStyle(.plain)
                            }

                            Button {
                                Task { _ = await viewModel.probeProxy(proxy, locale: locale) }
                            } label: {
                                Label(
                                    FabushiPlatformStrings.text(.settingsProxyProbeAction, locale: locale),
                                    systemImage: "speedometer"
                                )
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                                .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)

                            Spacer()

                            Button(role: .destructive) {
                                Task { _ = await viewModel.deleteProxy(id: proxy.id, locale: locale) }
                            } label: {
                                Image(systemName: "trash")
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.destructiveColor)
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }

            // MARK: Import `tg://proxy` / `tg://socks` Link
            Section(FabushiPlatformStrings.text(.settingsProxyImportTitle, locale: locale)) {
                TextField(
                    FabushiPlatformStrings.text(.settingsProxyImportPlaceholder, locale: locale),
                    text: $viewModel.importLinkDraft
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()

                Button {
                    Task { _ = await viewModel.importProxyFromDeepLink(locale: locale) }
                } label: {
                    Text(FabushiPlatformStrings.text(.settingsProxyImportAction, locale: locale))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(viewModel.importLinkDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionNetworkProxy, locale: locale))
        .task {
            await viewModel.refresh(locale: locale)
        }
        .onDisappear {
            viewModel.detachLifecycleHooks()
        }
    }
}
