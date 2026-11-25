import SwiftUI

// MARK: - Unified Global & In-Chat Search View (`#18`, Honest `BLK-08`)

/// Apple HIG unified search across the 8 real server/local backed categories:
/// `All`, `Chats`, `Messages`, `Media`, `Links`, `Files`, `Bots`, and `Mini Apps`.
///
/// Strictly enforces `BLK-08` (`FabushiMessagingCapabilityBoundaries.communityFoundationStatus`):
/// zero fabricated Community tabs, results, or `community.*` RPCs.
@MainActor
public struct FabushiUniversalSearchView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore
    public let inChatId: Int64?
    public let onDismiss: (() -> Void)?

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var queryText: String = ""
    @State private var selectedScope: FabushiSearchScope = .all

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil,
        inChatId: Int64? = nil,
        onDismiss: (() -> Void)? = nil
    ) {
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
        self.inChatId = inChatId
        self.onDismiss = onDismiss
    }

    private var availableScopes: [FabushiSearchScope] {
        if inChatId != nil {
            return [.messages, .media, .links, .files]
        }
        return FabushiSearchScope.allCases
    }

    public var body: some View {
        VStack(spacing: 0) {
            scopeFilterBar
            if store.isUsingOfflineSearchFallback && !queryText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                HStack(spacing: 8) {
                    Image(systemName: "internaldrive")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.accentColor)
                        .accessibilityHidden(true)
                    Text(theme.messagingString(.searchOfflineIndexBanner))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
                .background(theme.surfaceSecondary)
            }
            Divider().overlay(theme.separatorHairline)
            resultsArea
        }
        .background(theme.surfacePrimary)
        .navigationTitle(theme.messagingString(.searchTitle))
        .navigationBarTitleDisplayMode(.inline)
        .searchable(
            text: $queryText,
            placement: .navigationBarDrawer(displayMode: .always),
            prompt: Text(
                inChatId == nil
                    ? theme.messagingString(.searchPlaceholder)
                    : theme.messagingString(.searchInChatPlaceholder)
            )
        )
        .toolbar {
            if let onDismiss {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        onDismiss()
                    }
                }
            }
        }
        .onAppear {
            if inChatId != nil && !availableScopes.contains(selectedScope) {
                selectedScope = .messages
            }
        }
        .onChange(of: queryText) { _, newValue in
            store.scheduleDebouncedSearch(
                query: newValue,
                scope: selectedScope,
                inChatId: inChatId
            )
        }
        .onChange(of: selectedScope) { _, newScope in
            store.scheduleDebouncedSearch(
                query: queryText,
                scope: newScope,
                inChatId: inChatId
            )
        }
        .accessibilityIdentifier(
            inChatId == nil
                ? FabushiAccessibilityIDs.ChatList.searchField
                : FabushiAccessibilityIDs.Stage.searchBar
        )
    }

    // MARK: - 8-Category Scope Chip Bar

    private var scopeFilterBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(availableScopes) { scope in
                    let isSelected = selectedScope == scope
                    let title = theme.messagingString(scope.stringKey)
                    Button {
                        selectedScope = scope
                    } label: {
                        Text(title)
                            .fabushiFont(.caption)
                            .fontWeight(isSelected ? .semibold : .regular)
                            .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 7)
                            .background(isSelected ? theme.accentColor : theme.surfaceInput, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .fabushiMinimumTouchTarget()
                    .accessibilityLabel(title)
                    .accessibilityAddTraits(isSelected ? [.isSelected] : [])
                    .accessibilityIdentifier("fabushi.search.scope.\(scope.rawValue)")
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
        }
    }

    // MARK: - Results List

    @ViewBuilder
    private var resultsArea: some View {
        let trimmed = queryText.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            FabushiEmptyStateView(
                title: theme.messagingString(.searchEmptyPromptTitle),
                systemImage: "magnifyingglass",
                descriptionText: theme.messagingString(.searchEmptyPromptDescription),
                accessibilityID: "fabushi.search.emptyPrompt"
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if store.isSearching && store.searchResults.isEmpty {
            VStack(spacing: 12) {
                ProgressView()
                Text(theme.messagingString(.searchTitle))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if store.searchResults.isEmpty {
            FabushiEmptyStateView(
                title: theme.messagingString(.searchNoResultsTitle),
                systemImage: "doc.text.magnifyingglass",
                descriptionText: theme.messagingString(.searchNoResultsDescription),
                accessibilityID: "fabushi.search.noResults"
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            List {
                Section {
                    ForEach(store.searchResults) { result in
                        Button {
                            handleSelection(result)
                        } label: {
                            HStack(spacing: 12) {
                                resultIcon(for: result)
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack(spacing: 6) {
                                        Text(result.title)
                                            .fabushiFont(.headline)
                                            .foregroundStyle(theme.textPrimary)
                                            .lineLimit(1)
                                        Spacer(minLength: 4)
                                        if let ts = result.timestampMs, ts > 0 {
                                            Text(formattedDate(ts))
                                                .fabushiFont(.caption2)
                                                .foregroundStyle(theme.textMuted)
                                        }
                                    }
                                    if !result.snippet.isEmpty {
                                        Text(result.snippet)
                                            .fabushiFont(.subheadline)
                                            .foregroundStyle(theme.textSecondary)
                                            .lineLimit(2)
                                    }
                                }
                            }
                            .padding(.vertical, 4)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .fabushiMinimumTouchTarget()
                        .listRowBackground(theme.surfacePrimary)
                    }
                } header: {
                    FabushiSectionHeader(theme.messagingFormat(.searchResultsCountFormat, store.searchResults.count))
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.searchResultsList)
        }
    }

    @ViewBuilder
    private func resultIcon(for result: FabushiSearchResult) -> some View {
        switch result.kind {
        case .chat:
            FabushiAvatarBadge(peerId: result.chatId ?? 0, title: result.title, isOnline: false, isBot: false)
        case .bot:
            FabushiAvatarBadge(peerId: result.chatId ?? Int64(bitPattern: result.botUserId ?? 0), title: result.title, isOnline: false, isBot: true)
        case .miniApp:
            Image(systemName: "square.grid.2x2.fill")
                .fabushiFont(.title3)
                .foregroundStyle(theme.accentColor)
                .frame(width: 44, height: 44)
                .background(theme.accentColor.opacity(0.14), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityHidden(true)
        case .media:
            iconBadge(systemName: "photo.fill")
        case .link:
            iconBadge(systemName: "link")
        case .file:
            iconBadge(systemName: "doc.fill")
        case .sessionMemory:
            iconBadge(systemName: "brain.head.profile")
        case .message:
            iconBadge(systemName: "text.bubble.fill")
        }
    }

    private func iconBadge(systemName: String) -> some View {
        Image(systemName: systemName)
            .fabushiFont(.callout)
            .foregroundStyle(theme.accentColor)
            .frame(width: 40, height: 40)
            .background(theme.surfaceInput, in: Circle())
            .accessibilityHidden(true)
    }

    private func formattedDate(_ ms: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000.0)
        return date.formatted(.dateTime.month(.abbreviated).day())
    }

    private func handleSelection(_ result: FabushiSearchResult) {
        if store.isComposeActive {
            store.cancelComposeMode()
        }
        switch result.kind {
        case .miniApp:
            if let appId = result.miniAppId, !appId.isEmpty {
                router.navigate(to: .miniApp(id: appId), coordinator: coordinator)
            }
        case .chat, .message, .media, .link, .file, .bot:
            if let cid = result.chatId, cid != 0 {
                router.navigate(to: .chats(chatId: cid, messageId: result.messageId), coordinator: coordinator)
                Task { @MainActor in
                    await store.selectChat(cid)
                }
            }
        case .sessionMemory:
            router.navigate(to: .agentStudio(tab: "sessions"), coordinator: coordinator)
        }
        onDismiss?()
    }
}
