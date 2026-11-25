import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// SwiftUI representable hosting `FabushiBrowserWebViewRunner.webView` cleanly while recreating
/// only when switching isolated `WKWebsiteDataStore` profiles.
@MainActor
public struct FabushiBrowserWebViewRepresentable: UIViewRepresentable {
    public let webView: FabushiFocusGatedWKWebView

    public init(webView: FabushiFocusGatedWKWebView) {
        self.webView = webView
    }

    public func makeUIView(context: Context) -> FabushiFocusGatedWKWebView {
        webView
    }

    public func updateUIView(_ uiView: FabushiFocusGatedWKWebView, context: Context) {}
}

/// Adaptive Apple HIG Built-in Interactive Browser & Automation App Store workspace
/// (`Feature #28` + `D11-06..D11-09` + `D12-01` + `D16-01`).
///
/// - iPad: Side-by-side `WKWebView` viewport + 320pt 4-tab Inspector (`Copilot`, `Recorder`, `Session`, `Store`).
/// - iPhone: Full-bleed `WKWebView` + compact Liquid Glass URL/Control bar + `.sheet` 4-tab Inspector.
/// - Ready for 1-line M6 `ShellRoute.browser` / `Cmd+Shift+B` wiring and launched immediately from
///   M5 Remote Hub and Mini Apps Catalog.
@MainActor
public struct FabushiBrowserWorkspaceView: View {
    @State private var viewModel: FabushiBrowserViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @FocusState private var isURLBarFocused: Bool

    @State private var newProfileName: String = ""
    @State private var cookiePasteText: String = ""
    @State private var isCookieFileImporterPresented: Bool = false
    @State private var runTargetApp: FabushiAutomationScriptApp? = nil
    @State private var runParameterInputs: [String: String] = [:]

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }
    private var isRegularWidth: Bool { horizontalSizeClass == .regular }

    public init(
        initialURL: URL? = nil,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) {
        _viewModel = State(
            initialValue: FabushiBrowserViewModel(
                coordinator: coordinator,
                router: router,
                initialURL: initialURL
            )
        )
    }

    public var body: some View {
        Group {
            if isRegularWidth {
                HStack(spacing: 0) {
                    browserViewportColumn
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    Divider()
                    inspectorPanel
                        .frame(width: FabushiLayoutMetrics.inspectorIdealWidth)
                        .background(theme.surfaceSecondary)
                }
            } else {
                browserViewportColumn
            }
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.browserTitle, locale: locale))
        .onAppear {
            syncFocusPolicy()
        }
        .onChange(of: isURLBarFocused) { _, _ in
            syncFocusPolicy()
        }
        .onChange(of: viewModel.isInspectorSheetPresented) { _, _ in
            syncFocusPolicy()
        }
        .onChange(of: runTargetApp) { _, _ in
            syncFocusPolicy()
        }
        .onDisappear {
            viewModel.detachLifecycleHooks()
        }
        .task(id: viewModel.coordinator.accountScopeGeneration) {
            viewModel.reconcileAccountScopeIfNeeded()
            await viewModel.refreshInstalledApps()
        }
        .sheet(isPresented: $viewModel.isInspectorSheetPresented) {
            NavigationStack {
                inspectorPanel
                    .navigationTitle(FabushiPlatformStrings.text(.browserInspectorToggle, locale: locale))
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button(FabushiPlatformStrings.text(.done, locale: locale)) {
                                viewModel.isInspectorSheetPresented = false
                            }
                            .fabushiMinimumTouchTarget()
                        }
                    }
            }
        }
        .sheet(item: $runTargetApp) { app in
            parameterizedReplaySheet(app: app)
        }
        .fileImporter(
            isPresented: $isCookieFileImporterPresented,
            allowedContentTypes: [.json, .plainText]
        ) { result in
            switch result {
            case .success(let url):
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                if let data = try? Data(contentsOf: url) {
                    let rawText = String(decoding: data, as: UTF8.self)
                    Task {
                        _ = try? await viewModel.importCookies(from: rawText, locale: locale)
                    }
                }
            case .failure(let error):
                viewModel.router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            }
        }
    }

    private func syncFocusPolicy() {
        _ = viewModel.updateFocusPolicy(
            isViewportPresented: true,
            isURLBarFocused: isURLBarFocused,
            isInspectorModalPresented: !isRegularWidth && viewModel.isInspectorSheetPresented,
            hasBlockingModal: runTargetApp != nil
        )
    }

    // MARK: - Browser Viewport & Compact Liquid Glass URL/Control Bar

    private var browserViewportColumn: some View {
        VStack(spacing: 0) {
            urlControlBar
                .padding(.horizontal, 12)
                .padding(.vertical, 8)

            if viewModel.controlMode != .idle {
                controlGateBanner
                    .padding(.horizontal, 12)
                    .padding(.bottom, 8)
            }

            Divider()

            FabushiBrowserWebViewRepresentable(webView: viewModel.runner.webView)
                .id(viewModel.activeProfileId)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var urlControlBar: some View {
        HStack(spacing: 8) {
            Button {
                viewModel.runner.goBack()
            } label: {
                Image(systemName: "chevron.backward")
                    .fabushiFont(.subheadline)
            }
            .disabled(!viewModel.canGoBack)
            .fabushiMinimumTouchTarget()
            .accessibilityLabel(FabushiPlatformStrings.text(.browserBack, locale: locale))

            Button {
                viewModel.runner.goForward()
            } label: {
                Image(systemName: "chevron.forward")
                    .fabushiFont(.subheadline)
            }
            .disabled(!viewModel.canGoForward)
            .fabushiMinimumTouchTarget()
            .accessibilityLabel(FabushiPlatformStrings.text(.browserForward, locale: locale))

            HStack(spacing: 6) {
                Image(systemName: viewModel.currentURLString.hasPrefix("https://") ? "lock.fill" : "globe")
                    .fabushiFont(.caption)
                    .foregroundStyle(viewModel.currentURLString.hasPrefix("https://") ? theme.successColor : theme.textSecondary)

                TextField(
                    FabushiPlatformStrings.text(.browserUrlPlaceholder, locale: locale),
                    text: $viewModel.addressBarText
                )
                .fabushiFont(.subheadline)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .focused($isURLBarFocused)
                .onSubmit {
                    isURLBarFocused = false
                    _ = viewModel.navigateFromAddressBar(locale: locale)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous))

            Button {
                if viewModel.isPageLoading {
                    viewModel.runner.stopLoading()
                } else {
                    viewModel.runner.reload()
                }
            } label: {
                Image(systemName: viewModel.isPageLoading ? "xmark" : "arrow.clockwise")
                    .fabushiFont(.subheadline)
            }
            .fabushiMinimumTouchTarget()
            .accessibilityLabel(
                viewModel.isPageLoading
                    ? FabushiPlatformStrings.text(.browserStopLoading, locale: locale)
                    : FabushiPlatformStrings.text(.browserReload, locale: locale)
            )

            if !isRegularWidth {
                Button {
                    isURLBarFocused = false
                    viewModel.isInspectorSheetPresented = true
                } label: {
                    Image(systemName: "sidebar.right")
                        .fabushiFont(.subheadline)
                }
                .fabushiMinimumTouchTarget()
                .accessibilityLabel(FabushiPlatformStrings.text(.browserInspectorToggle, locale: locale))
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .fabushiLiquidGlass(cornerRadius: 16, isInteractive: true)
    }

    private var controlGateBanner: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(viewModel.controlMode == .cancelled ? theme.textMuted : theme.accentColor)
                .frame(width: 8, height: 8)
            Text(viewModel.controlMode.badgeTitle(locale: locale))
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)

            if let progress = viewModel.replayStepProgress {
                Text(FabushiPlatformStrings.stepProgress(current: progress.current, total: progress.total, locale: locale))
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.textSecondary)
            }

            Spacer()

            switch viewModel.controlMode {
            case .aiControlling, .waitingForTakeover:
                Button(FabushiPlatformStrings.text(.browserTakeOverAction, locale: locale)) {
                    viewModel.takeOverByUser()
                }
                .fabushiFont(.caption)
                .fabushiMinimumTouchTarget()

                Button(FabushiPlatformStrings.text(.browserCancelAiAction, locale: locale)) {
                    viewModel.cancelAiControl()
                }
                .fabushiFont(.caption)
                .foregroundStyle(theme.destructiveColor)
                .fabushiMinimumTouchTarget()

            case .userTakeover:
                Button(FabushiPlatformStrings.text(.browserResumeAiAction, locale: locale)) {
                    viewModel.resumeAiControl()
                }
                .fabushiFont(.caption)
                .fabushiMinimumTouchTarget()

                Button(FabushiPlatformStrings.text(.browserCancelAiAction, locale: locale)) {
                    viewModel.cancelAiControl()
                }
                .fabushiFont(.caption)
                .foregroundStyle(theme.destructiveColor)
                .fabushiMinimumTouchTarget()

            case .idle, .cancelled:
                EmptyView()
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    // MARK: - 4-Tab Inspector (`Copilot`, `Recorder`, `Session`, `Store`)

    private var inspectorPanel: some View {
        VStack(spacing: 0) {
            Picker("", selection: $viewModel.selectedInspectorTab) {
                ForEach(FabushiBrowserInspectorTab.allCases) { tab in
                    Text(tab.displayTitle(locale: locale)).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .padding(12)

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    switch viewModel.selectedInspectorTab {
                    case .copilot:
                        copilotTab
                    case .recorder:
                        recorderTab
                    case .session:
                        sessionTab
                    case .store:
                        storeTab
                    }
                }
                .padding(14)
            }
        }
    }

    // MARK: Tab 1 — Copilot & Control Gate

    private var copilotTab: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(viewModel.controlMode.badgeTitle(locale: locale))
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)

            HStack(spacing: 8) {
                Button(FabushiPlatformStrings.text(.browserTakeOverAction, locale: locale)) {
                    viewModel.takeOverByUser()
                }
                .fabushiFont(.caption)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(theme.surfaceElevated, in: Capsule())
                .fabushiMinimumTouchTarget()

                Button(FabushiPlatformStrings.text(.browserResumeAiAction, locale: locale)) {
                    viewModel.resumeAiControl()
                }
                .fabushiFont(.caption)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(theme.accentColor.opacity(0.18), in: Capsule())
                .fabushiMinimumTouchTarget()

                Button(FabushiPlatformStrings.text(.browserCancelAiAction, locale: locale)) {
                    viewModel.cancelAiControl()
                }
                .fabushiFont(.caption)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(theme.destructiveColor.opacity(0.14), in: Capsule())
                .foregroundStyle(theme.destructiveColor)
                .fabushiMinimumTouchTarget()
            }

            if !viewModel.replayLogs.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(viewModel.replayLogs.enumerated()), id: \.offset) { _, line in
                        Text(line)
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

    // MARK: Tab 2 — DOM Recorder & Secret-Free Distillation

    private var recorderTab: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                Button {
                    if viewModel.isRecording {
                        viewModel.stopRecording()
                    } else {
                        viewModel.startRecording()
                    }
                } label: {
                    Label(
                        viewModel.isRecording
                            ? FabushiPlatformStrings.text(.browserStopRecording, locale: locale)
                            : FabushiPlatformStrings.text(.browserStartRecording, locale: locale),
                        systemImage: viewModel.isRecording ? "stop.circle.fill" : "record.circle.fill"
                    )
                    .fabushiFont(.subheadline)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(viewModel.isRecording ? theme.destructiveColor : theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .foregroundStyle(Color.white)
                }
                .fabushiMinimumTouchTarget()

                Button {
                    viewModel.clearTrace()
                } label: {
                    Text(FabushiPlatformStrings.text(.browserClearTrace, locale: locale))
                        .fabushiFont(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 10)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                }
                .fabushiMinimumTouchTarget()
            }

            Button {
                _ = try? viewModel.distillTraceToScript()
            } label: {
                Label(
                    FabushiPlatformStrings.text(.browserDistillScript, locale: locale),
                    systemImage: "wand.and.stars"
                )
                .fabushiFont(.subheadline)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
                .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                .foregroundStyle(theme.textPrimary)
            }
            .disabled(viewModel.currentTrace.actions.isEmpty)
            .fabushiMinimumTouchTarget()

            if !viewModel.lastSecretLeakFindings.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text(FabushiPlatformStrings.text(.browserStoreSecretLeakBlocked, locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.destructiveColor)
                    ForEach(viewModel.lastSecretLeakFindings) { finding in
                        Text("\(finding.fieldPath): \(finding.summary)")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
                .padding(10)
                .background(theme.destructiveColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }

            Text(FabushiPlatformStrings.text(.browserRecordedActionsTitle, locale: locale))
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.textPrimary)

            if viewModel.currentTrace.actions.isEmpty {
                Text(FabushiPlatformStrings.text(.browserNoRecordedActions, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            } else {
                ForEach(viewModel.currentTrace.actions) { item in
                    HStack(spacing: 8) {
                        Text("#\(item.seq)")
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textMuted)
                        Text(item.summaryText)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textPrimary)
                            .lineLimit(2)
                        Spacer()
                        if item.isSecretRedacted {
                            Text(FabushiPlatformStrings.text(.browserSecretRedactedBadge, locale: locale))
                                .fabushiFont(.caption2)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 3)
                                .background(theme.warningColor.opacity(0.2), in: Capsule())
                                .foregroundStyle(theme.warningColor)
                        }
                    }
                    .padding(8)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }
        }
    }

    // MARK: Tab 3 — Isolated Session Profiles, Cookies & E2EE Vault Sync

    private var sessionTab: some View {
        VStack(alignment: .leading, spacing: 12) {
            // Honest iOS Sandbox Caption (NO local Chromium / NO Chrome SQLite scraping)
            Text(FabushiPlatformStrings.text(.browserSandboxCaption, locale: locale))
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
                .padding(10)
                .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            Text(FabushiPlatformStrings.text(.browserSessionProfilesTitle, locale: locale))
                .fabushiFont(.headline)

            ForEach(viewModel.profiles) { profile in
                let isSelected = profile.profileKey == viewModel.activeProfileId
                Button {
                    viewModel.selectProfile(profile)
                } label: {
                    HStack {
                        Image(systemName: profile.isEphemeral ? "eye.slash" : "person.crop.circle")
                        Text(profile.isEphemeral ? FabushiPlatformStrings.text(.browserEphemeralProfile, locale: locale) : profile.displayName)
                            .fabushiFont(.subheadline)
                        Spacer()
                        if isSelected {
                            Image(systemName: "checkmark")
                                .foregroundStyle(theme.accentColor)
                        }
                    }
                    .padding(10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
                .fabushiMinimumTouchTarget()
            }

            HStack(spacing: 8) {
                TextField(FabushiPlatformStrings.text(.browserNewProfile, locale: locale), text: $newProfileName)
                    .fabushiFont(.caption)
                    .padding(8)
                    .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                Button(FabushiPlatformStrings.text(.save, locale: locale)) {
                    _ = viewModel.createNamedProfile(displayName: newProfileName)
                    newProfileName = ""
                }
                .fabushiFont(.caption)
                .fabushiMinimumTouchTarget()
            }

            Divider()

            Text(FabushiPlatformStrings.text(.browserImportCookiesTitle, locale: locale))
                .fabushiFont(.subheadline)

            TextField(
                FabushiPlatformStrings.text(.browserImportCookiesPlaceholder, locale: locale),
                text: $cookiePasteText,
                axis: .vertical
            )
            .lineLimit(3...5)
            .fabushiFont(.monospacedCaption)
            .padding(8)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 8, style: .continuous))

            HStack(spacing: 8) {
                Button(FabushiPlatformStrings.text(.browserImportCookiesPasteAction, locale: locale)) {
                    let text = cookiePasteText
                    Task {
                        _ = try? await viewModel.importCookies(from: text, locale: locale)
                    }
                }
                .fabushiFont(.caption)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(theme.accentColor, in: Capsule())
                .foregroundStyle(theme.accentForegroundColor)
                .fabushiMinimumTouchTarget()

                Button(FabushiPlatformStrings.text(.browserImportCookiesFileAction, locale: locale)) {
                    isCookieFileImporterPresented = true
                }
                .fabushiFont(.caption)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(theme.surfaceElevated, in: Capsule())
                .fabushiMinimumTouchTarget()
            }

            // E2EE Vault Backup / Restore (`browser_cookie`)
            HStack(spacing: 8) {
                Button {
                    Task {
                        _ = try? await viewModel.backupActiveProfileToVault(pushToServer: true, locale: locale)
                    }
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.browserVaultBackupAction, locale: locale),
                        systemImage: "lock.icloud"
                    )
                    .fabushiFont(.caption)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
                .fabushiMinimumTouchTarget()

                Button {
                    if let blob = viewModel.lastVaultBackupBlob {
                        Task {
                            _ = try? await viewModel.restoreProfileFromVaultBlob(blob, locale: locale)
                        }
                    }
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.browserVaultRestoreAction, locale: locale),
                        systemImage: "arrow.down.doc"
                    )
                    .fabushiFont(.caption)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
                .disabled(viewModel.lastVaultBackupBlob == nil)
                .fabushiMinimumTouchTarget()
            }

            Button(role: .destructive) {
                Task { await viewModel.clearActiveProfileData(locale: locale) }
            } label: {
                Text(FabushiPlatformStrings.text(.browserClearCookiesAction, locale: locale))
                    .fabushiFont(.caption)
            }
            .fabushiMinimumTouchTarget()
        }
    }

    // MARK: Tab 4 — Automation App Store

    private var storeTab: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                TextField(
                    FabushiPlatformStrings.text(.browserStoreSearchPlaceholder, locale: locale),
                    text: $viewModel.storeSearchQuery
                )
                .fabushiFont(.caption)
                .padding(8)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .onSubmit {
                    Task { await viewModel.searchStore() }
                }

                Button(FabushiPlatformStrings.text(.refresh, locale: locale)) {
                    Task {
                        await viewModel.searchStore()
                        await viewModel.refreshInstalledApps()
                    }
                }
                .fabushiFont(.caption)
                .fabushiMinimumTouchTarget()
            }

            if let draft = viewModel.distilledDraft {
                Button {
                    Task {
                        _ = try? await viewModel.publishScriptApp(draft, locale: locale)
                    }
                } label: {
                    Label(
                        "\(FabushiPlatformStrings.text(.browserStorePublishDraft, locale: locale)): \(draft.name)",
                        systemImage: "arrow.up.circle.fill"
                    )
                    .fabushiFont(.caption)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                    .background(theme.accentColor, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .foregroundStyle(theme.accentForegroundColor)
                }
                .fabushiMinimumTouchTarget()
            }

            if !viewModel.installedApps.isEmpty {
                Text(FabushiPlatformStrings.text(.browserStoreInstalledSection, locale: locale))
                    .fabushiFont(.subheadline)
                ForEach(viewModel.installedApps) { installed in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(installed.name)
                                .fabushiFont(.subheadline)
                            Spacer()
                            Text(FabushiPlatformStrings.text(.browserStoreZeroLlmBadge, locale: locale))
                                .fabushiFont(.caption2)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 3)
                                .background(theme.successColor.opacity(0.18), in: Capsule())
                                .foregroundStyle(theme.successColor)
                        }
                        HStack(spacing: 8) {
                            Button(FabushiPlatformStrings.text(.browserStoreRunScript, locale: locale)) {
                                if let script = installed.decodedScriptApp() {
                                    runParameterInputs = [:]
                                    runTargetApp = script
                                }
                            }
                            .fabushiFont(.caption)
                            .fabushiMinimumTouchTarget()

                            Button(FabushiPlatformStrings.text(.uninstall, locale: locale), role: .destructive) {
                                Task { _ = await viewModel.uninstallApp(appId: installed.appId, locale: locale) }
                            }
                            .fabushiFont(.caption)
                            .fabushiMinimumTouchTarget()
                        }
                    }
                    .padding(10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }

            Text(FabushiPlatformStrings.text(.browserStoreCatalogSection, locale: locale))
                .fabushiFont(.subheadline)

            if viewModel.storeSummaries.isEmpty {
                Text(FabushiPlatformStrings.text(.browserStoreNoAppsSubtitle, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            } else {
                ForEach(viewModel.storeSummaries) { summary in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(summary.name)
                            .fabushiFont(.subheadline)
                        Text(summary.description)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                        HStack {
                            Text(FabushiPlatformStrings.stepsCount(summary.stepCount, locale: locale))
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                            Spacer()
                            Button(
                                summary.isInstalled
                                    ? FabushiPlatformStrings.text(.installed, locale: locale)
                                    : FabushiPlatformStrings.text(.install, locale: locale)
                            ) {
                                Task { _ = await viewModel.installApp(appId: summary.appId, locale: locale) }
                            }
                            .fabushiFont(.caption)
                            .fabushiMinimumTouchTarget()
                        }
                    }
                    .padding(10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }
        }
    }

    // MARK: - Parameterized Zero-LLM Replay Sheet

    private func parameterizedReplaySheet(app: FabushiAutomationScriptApp) -> some View {
        NavigationStack {
            Form {
                Section(FabushiPlatformStrings.text(.browserStoreParametersTitle, locale: locale)) {
                    if app.parameters.isEmpty {
                        Text(FabushiPlatformStrings.stepsCount(app.steps.count, locale: locale))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    } else {
                        ForEach(app.parameters) { param in
                            VStack(alignment: .leading, spacing: 4) {
                                if param.isSecret {
                                    SecureField(param.name, text: Binding(
                                        get: { runParameterInputs[param.name] ?? "" },
                                        set: { runParameterInputs[param.name] = $0 }
                                    ))
                                    Text(FabushiPlatformStrings.text(.browserStoreSecretParamHint, locale: locale))
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.textSecondary)
                                } else {
                                    TextField(param.name, text: Binding(
                                        get: { runParameterInputs[param.name] ?? param.defaultValue ?? "" },
                                        set: { runParameterInputs[param.name] = $0 }
                                    ))
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle(app.name)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.cancel, locale: locale)) {
                        runTargetApp = nil
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(FabushiPlatformStrings.text(.run, locale: locale)) {
                        let params = runParameterInputs
                        runTargetApp = nil
                        Task {
                            _ = await viewModel.runScriptApp(app, parameterValues: params, locale: locale)
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
