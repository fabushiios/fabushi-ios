import SwiftUI
import WebKit

/// SwiftUI bridge mounting an existing pooled `FabushiFocusGatedWKWebView` in-place
/// (zero reload when restoring from floating PiP!).
@MainActor
public struct FabushiPooledMiniAppWebViewRepresentable: UIViewRepresentable {
    public let webView: FabushiFocusGatedWKWebView

    public init(webView: FabushiFocusGatedWKWebView) {
        self.webView = webView
    }

    public func makeUIView(context: Context) -> FabushiFocusGatedWKWebView {
        webView
    }

    public func updateUIView(_ uiView: FabushiFocusGatedWKWebView, context: Context) {}
}

/// Unified Apple HIG Mini Apps Host Sheet (`makeMiniAppHostSheetView`).
///
/// Wraps in its OWN `NavigationStack` because `FabushiAdaptiveShellView` presents it as a modal
/// `.sheet`. Handles `"catalog"`, `"global_dharma"` / `"global-dharma"`, and individual Mini App
/// sessions with mandatory ToS gating, WebMCP status/takeover controls, unified `minimize()`,
/// `CreateManagedBotBox` `originBoxId` close-race guard, and `D16-01` Built-in Browser launcher.
@MainActor
public struct FabushiMiniAppHostSheetView: View {
    public let miniAppId: String?
    @State private var viewModel: FabushiMiniAppHostViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @FocusState private var isSearchFocused: Bool

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }
    private var isCompact: Bool { horizontalSizeClass != .regular }

    public init(
        miniAppId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        sessionPool: FabushiMiniAppSessionPool = .shared
    ) {
        self.miniAppId = miniAppId
        _viewModel = State(
            initialValue: FabushiMiniAppHostViewModel(
                coordinator: coordinator,
                router: router,
                sessionPool: sessionPool
            )
        )
    }

    public var body: some View {
        NavigationStack {
            Group {
                if viewModel.isGlobalDharmaPresented {
                    FabushiGlobalDharmaView(viewModel: viewModel)
                } else if let session = viewModel.activeSession {
                    activeMiniAppContainer(session: session)
                } else {
                    catalogView
                }
            }
            .background(theme.surfacePrimary)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    if viewModel.isGlobalDharmaPresented || viewModel.activeSession != nil {
                        Button(FabushiPlatformStrings.text(.back, locale: locale)) {
                            if viewModel.activeSession != nil {
                                viewModel.minimize(
                                    reason: .explicitMinimizeButton,
                                    isCompact: isCompact,
                                    locale: locale
                                )
                            } else {
                                viewModel.isGlobalDharmaPresented = false
                            }
                        }
                        .fabushiMinimumTouchTarget()
                    } else {
                        Button(FabushiPlatformStrings.text(.close, locale: locale)) {
                            viewModel.router.dismissActiveModalSheet(coordinator: viewModel.coordinator)
                        }
                        .fabushiMinimumTouchTarget()
                        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.closeButton)
                    }
                }

                ToolbarItemGroup(placement: .primaryAction) {
                    if viewModel.activeSession != nil {
                        Button {
                            viewModel.minimize(
                                reason: .explicitMinimizeButton,
                                isCompact: isCompact,
                                locale: locale
                            )
                        } label: {
                            Image(systemName: "pip.enter")
                        }
                        .fabushiMinimumTouchTarget()
                        .accessibilityLabel(FabushiPlatformStrings.text(.miniAppMinimizePiP, locale: locale))
                        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.minimizePiPButton)

                        Button {
                            viewModel.closeActiveSession(isCompact: isCompact)
                        } label: {
                            Image(systemName: "xmark")
                        }
                        .fabushiMinimumTouchTarget()
                        .accessibilityLabel(FabushiPlatformStrings.text(.miniAppCloseSession, locale: locale))
                        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.closeButton)
                    } else {
                        Button {
                            _ = viewModel.openCreateManagedBotBox()
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.miniAppCreateManagedBot, locale: locale),
                                systemImage: "plus.circle.fill"
                            )
                            .fabushiFont(.subheadline)
                        }
                        .fabushiMinimumTouchTarget()
                        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.launcherButton)
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.root)
        .task(id: viewModel.coordinator.accountScopeGeneration) {
            await viewModel.handleInitialMiniAppId(miniAppId, locale: locale)
        }
        .onChange(of: isSearchFocused) { _, focused in
            _ = viewModel.updateFocusPolicy(isPresented: true, isHostInputFocused: focused)
        }
        .onDisappear {
            viewModel.detachLifecycleHooks()
        }
        .sheet(item: $viewModel.pendingTosPrompt) { prompt in
            tosConsentModal(prompt: prompt)
        }
        .sheet(isPresented: $viewModel.isCreateBotSheetPresented) {
            if let boxId = viewModel.activeCreateBoxId {
                FabushiCreateManagedBotSheetView(originBoxId: boxId, viewModel: viewModel)
            }
        }
        .sheet(isPresented: $viewModel.isBrowserSheetPresented) {
            NavigationStack {
                FabushiBrowserWorkspaceView(
                    initialURL: nil,
                    coordinator: viewModel.coordinator,
                    router: viewModel.router
                )
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(FabushiPlatformStrings.text(.close, locale: locale)) {
                            viewModel.isBrowserSheetPresented = false
                        }
                        .fabushiMinimumTouchTarget()
                    }
                }
            }
        }
    }

    // MARK: - Mini Apps Catalog View

    private var catalogView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                // Search Bar + Filter
                TextField(
                    FabushiPlatformStrings.text(.miniAppCatalogSearchPlaceholder, locale: locale),
                    text: $viewModel.searchQuery
                )
                .fabushiFont(.body)
                .focused($isSearchFocused)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous))
                .onSubmit {
                    Task { await viewModel.refreshCatalog() }
                }

                Picker("", selection: $viewModel.selectedFilter) {
                    ForEach(FabushiMiniAppCatalogFilter.allCases) { filter in
                        Text(filter.displayTitle(locale: locale)).tag(filter)
                    }
                }
                .pickerStyle(.segmented)

                if let blocked = viewModel.tosBlockedMessage {
                    HStack(spacing: 8) {
                        Image(systemName: "hand.raised.slash.fill")
                            .foregroundStyle(theme.destructiveColor)
                        Text(blocked)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.destructiveColor)
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.destructiveColor.opacity(0.12), in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.tosBlockedBanner)
                }

                // Prominent 全球法布施 (Global Dharma) Card
                Button {
                    viewModel.isGlobalDharmaPresented = true
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "globe.asia.australia.fill")
                            .fabushiFont(.title2)
                            .foregroundStyle(theme.accentColor)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(FabushiPlatformStrings.text(.miniAppGlobalDharmaTitle, locale: locale))
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Text(FabushiPlatformStrings.text(.miniAppGlobalDharmaSubtitle, locale: locale))
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                                .multilineTextAlignment(.leading)
                        }
                        Spacer()
                        Text(FabushiPlatformStrings.text(.open, locale: locale))
                            .fabushiFont(.caption)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(theme.accentColor, in: Capsule())
                            .foregroundStyle(theme.accentForegroundColor)
                    }
                    .padding(14)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                            .strokeBorder(theme.accentColor.opacity(0.35), lineWidth: 1)
                    )
                }
                .fabushiMinimumTouchTarget()
                .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.globalDharmaCard)

                // D16-01 Prominent Built-in Browser & Apps Launcher Card
                Button {
                    viewModel.isBrowserSheetPresented = true
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "safari.fill")
                            .fabushiFont(.title2)
                            .foregroundStyle(theme.accentColor)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(FabushiPlatformStrings.text(.builtInBrowserTitle, locale: locale))
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Text(FabushiPlatformStrings.text(.builtInBrowserSubtitle, locale: locale))
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                                .multilineTextAlignment(.leading)
                        }
                        Spacer()
                        Image(systemName: "chevron.right")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textMuted)
                    }
                    .padding(14)
                    .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                            .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
                    )
                }
                .fabushiMinimumTouchTarget()

                // Catalog List
                LazyVStack(spacing: 10) {
                    if viewModel.filteredApps.isEmpty {
                        FabushiEmptyStateView(
                            title: FabushiPlatformStrings.text(.miniAppEmptyCatalogTitle, locale: locale),
                            systemImage: "square.grid.2x2",
                            descriptionText: FabushiPlatformStrings.text(.miniAppEmptyCatalogSubtitle, locale: locale),
                            accessibilityID: "fabushi.miniapp.emptyState"
                        )
                    } else {
                        ForEach(viewModel.filteredApps) { app in
                            miniAppRow(app)
                        }
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.catalogList)
            }
            .padding(16)
        }
        .navigationTitle(FabushiPlatformStrings.text(.miniAppHubTitle, locale: locale))
        .refreshable {
            await viewModel.refreshCatalog()
        }
    }

    private func miniAppRow(_ app: FabushiMiniAppEntry) -> some View {
        Button {
            Task {
                _ = await viewModel.launchMiniApp(app, startMinimized: false, locale: locale)
            }
        } label: {
            HStack(spacing: 12) {
                Text(app.iconEmoji.isEmpty ? "🧩" : app.iconEmoji)
                    .fabushiFont(.title2)
                    .frame(width: 44, height: 44)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                VStack(alignment: .leading, spacing: 3) {
                    Text(app.title.isEmpty ? app.shortName : app.title)
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)
                    Text(app.description.isEmpty ? "@\(app.botUsername)" : app.description)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                        .lineLimit(2)
                }

                Spacer()

                Text(FabushiPlatformStrings.text(.open, locale: locale))
                    .fabushiFont(.caption)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.surfaceElevated, in: Capsule())
                    .foregroundStyle(theme.accentColor)
            }
            .padding(12)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
        .fabushiMinimumTouchTarget()
    }

    // MARK: - Active Mini App WebView + WebMCP Controls

    private func activeMiniAppContainer(session: FabushiMiniAppActiveSession) -> some View {
        VStack(spacing: 0) {
            // WebMCP Status Strip
            HStack(spacing: 10) {
                Text(session.controlMode.displayTitle(locale: locale))
                    .fabushiFont(.caption)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(theme.accentColor.opacity(0.16), in: Capsule())
                    .foregroundStyle(theme.accentColor)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.webMcpStatusBadge)

                Spacer()

                if session.controlMode == .aiControlling || session.controlMode == .waitingForTakeover {
                    Button(FabushiPlatformStrings.text(.miniAppWebMcpTakeOverButton, locale: locale)) {
                        Task {
                            _ = await viewModel.setWebMcpControlMode(
                                appKey: session.app.appKey,
                                mode: .userTakeover,
                                locale: locale
                            )
                        }
                    }
                    .fabushiFont(.caption)
                    .fabushiMinimumTouchTarget()
                } else if session.controlMode == .userTakeover {
                    Button(FabushiPlatformStrings.text(.miniAppWebMcpResumeAiButton, locale: locale)) {
                        Task {
                            _ = await viewModel.setWebMcpControlMode(
                                appKey: session.app.appKey,
                                mode: .aiControlling,
                                locale: locale
                            )
                        }
                    }
                    .fabushiFont(.caption)
                    .fabushiMinimumTouchTarget()
                }

                if let companionChatId = session.companionChatId {
                    Button {
                        // D10-12: Opening companion chat MUST pass through unified minimize()!
                        viewModel.minimize(reason: .openCompanionChat, isCompact: isCompact, locale: locale)
                        viewModel.router.navigate(
                            to: .chats(chatId: companionChatId, messageId: nil),
                            coordinator: viewModel.coordinator
                        )
                    } label: {
                        Image(systemName: "bubble.left.and.bubble.right")
                            .fabushiFont(.caption)
                    }
                    .fabushiMinimumTouchTarget()
                    .accessibilityLabel(FabushiPlatformStrings.text(.miniAppOpenCompanionChat, locale: locale))
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(theme.surfaceTopBar)

            if let errorMsg = viewModel.webMcpErrorMessage {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(theme.destructiveColor)
                    Text(errorMsg)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.destructiveColor)
                    Spacer()
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(theme.destructiveColor.opacity(0.12))
                .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.webMcpErrorBanner)
            }

            if let pooled = viewModel.activePooledSession {
                FabushiPooledMiniAppWebViewRepresentable(webView: pooled.webView)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.webView)
            } else {
                FabushiEmptyStateView(
                    title: session.app.title,
                    systemImage: "safari",
                    descriptionText: session.resolvedURL?.absoluteString ?? session.app.description,
                    accessibilityID: FabushiAccessibilityIDs.MiniApp.webView
                )
            }
        }
        .navigationTitle(session.app.title)
    }

    // MARK: - Mandatory ToS Modal

    private func tosConsentModal(prompt: FabushiMiniAppTosPromptState) -> some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 16) {
                Text(prompt.app.title.isEmpty ? prompt.app.shortName : prompt.app.title)
                    .fabushiFont(.title3)
                    .foregroundStyle(theme.textPrimary)

                Text(FabushiPlatformStrings.text(.miniAppTosMessage, locale: locale))
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textSecondary)

                if !prompt.app.privacyPolicyUrl.isEmpty {
                    Text(prompt.app.privacyPolicyUrl)
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(theme.textLink)
                }

                Spacer()

                Button {
                    Task {
                        _ = await viewModel.acceptPendingTos(locale: locale)
                    }
                } label: {
                    Text(FabushiPlatformStrings.text(.miniAppTosAccept, locale: locale))
                        .fabushiFont(.headline)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                        .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                        .foregroundStyle(theme.accentForegroundColor)
                }
                .fabushiMinimumTouchTarget()
                .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.tosAcceptButton)

                Button(role: .destructive) {
                    viewModel.declinePendingTos(locale: locale)
                } label: {
                    Text(FabushiPlatformStrings.text(.miniAppTosDecline, locale: locale))
                        .fabushiFont(.subheadline)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                }
                .fabushiMinimumTouchTarget()
                .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.tosDeclineButton)
            }
            .padding(20)
            .navigationTitle(FabushiPlatformStrings.text(.miniAppTosTitle, locale: locale))
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.MiniApp.tosModal)
        }
    }
}
