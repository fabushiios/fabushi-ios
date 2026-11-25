import SwiftUI

// MARK: - Unified Apple HIG Chat List View (`#10`, `D13-01..05`, `D14-04`, `D14-05`)

/// Unified native Apple HIG conversation list for Humans, Bots, Groups, Channels, Personal Notes,
/// and Forum Topics.
///
/// Layout invariants:
/// - NEVER nests a second `NavigationStack` (the shell owns the enclosing `NavigationStack`).
/// - Keeps compact iPhone actions in `.topBarTrailing` / inline header chrome so they never
///   collide with shell `.topBarLeading` or `.principal`.
@MainActor
public struct FabushiChatListView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var quickFilterText: String = ""
    @State private var isSearchSheetPresented: Bool = false
    @State private var isFolderEditorPresented: Bool = false
    @State private var editingFolder: FabushiFolder? = nil
    @State private var activeTopicsChat: FabushiMessagingChat? = nil
    @State private var pendingDeleteChat: FabushiMessagingChat? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil
    ) {
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
    }

    private var displayedChats: [FabushiMessagingChat] {
        store.filteredChats(searchText: quickFilterText)
    }

    private var shouldShowLoadingSkeleton: Bool {
        store.isLoadingDialogs
            && store.chats.isEmpty
            && !store.hasSyncedDialogs
            && quickFilterText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public var body: some View {
        VStack(spacing: 0) {
            inlineHeaderControls
            if !store.stories.isEmpty {
                storiesTrayStrip
            }
            folderPillBar
            statusBannersStack
            Divider().overlay(theme.separatorHairline)
            mainContentArea
        }
        .background(theme.surfacePrimary)
        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.root)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    if store.isComposeActive {
                        store.cancelComposeMode()
                    }
                    isSearchSheetPresented = true
                } label: {
                    Image(systemName: "magnifyingglass")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.localized("hig.common.search"))

                Button {
                    store.beginComposeMode()
                    router.navigate(to: .compose, coordinator: coordinator)
                } label: {
                    Image(systemName: "square.and.pencil")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.localized("hig.common.compose"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.composeButton)
            }
        }
        .task(id: coordinator.accountScopeGeneration) {
            _ = store.reconcileAccountScopeIfNeeded()
            store.startEventLoopIfNeeded()
            _ = store.hydrateFrame0()
            await store.refreshWorkspace(trigger: .initialAppear)
        }
        .sheet(isPresented: $isSearchSheetPresented) {
            NavigationStack {
                FabushiUniversalSearchView(
                    coordinator: coordinator,
                    router: router,
                    store: store,
                    inChatId: nil,
                    onDismiss: { isSearchSheetPresented = false }
                )
            }
        }
        .sheet(isPresented: $isFolderEditorPresented) {
            FabushiFolderEditorSheet(
                existingFolder: editingFolder,
                onSave: { folder in
                    Task { @MainActor in
                        _ = try? await store.saveFolder(folder)
                        isFolderEditorPresented = false
                        editingFolder = nil
                    }
                },
                onDelete: { folderId in
                    Task { @MainActor in
                        try? await store.deleteFolder(folderId: folderId)
                        isFolderEditorPresented = false
                        editingFolder = nil
                    }
                },
                onCancel: {
                    isFolderEditorPresented = false
                    editingFolder = nil
                }
            )
        }
        .sheet(item: $activeTopicsChat) { chat in
            NavigationStack {
                FabushiForumTopicsSheetView(
                    chat: chat,
                    store: store,
                    onSelectTopic: { _ in
                        activeTopicsChat = nil
                        openChat(chat)
                    },
                    onClose: { activeTopicsChat = nil }
                )
            }
        }
        .confirmationDialog(
            theme.messagingString(.deleteChatAction),
            isPresented: Binding(
                get: { pendingDeleteChat != nil },
                set: { if !$0 { pendingDeleteChat = nil } }
            ),
            titleVisibility: .visible
        ) {
            if let chat = pendingDeleteChat {
                Button(theme.messagingString(.deleteChatAction), role: .destructive) {
                    Task { @MainActor in
                        try? await store.deleteChat(chatId: chat.chatId)
                        pendingDeleteChat = nil
                    }
                }
            }
            Button(theme.localized("hig.common.cancel"), role: .cancel) {
                pendingDeleteChat = nil
            }
        }
    }

    // MARK: - Inline Header & Search Bar

    private var inlineHeaderControls: some View {
        HStack(spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textMuted)
                    .accessibilityHidden(true)

                TextField(
                    theme.messagingString(.searchPlaceholder),
                    text: $quickFilterText
                )
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.textPrimary)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.searchField)

                if !quickFilterText.isEmpty {
                    Button {
                        quickFilterText = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textMuted)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized("hig.common.close"))
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                theme.surfaceInput,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous)
            )

            Button {
                if store.isComposeActive {
                    store.cancelComposeMode()
                }
                isSearchSheetPresented = true
            } label: {
                Image(systemName: "sparkle.magnifyingglass")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.messagingString(.searchTitle))

            if horizontalSizeClass == .regular {
                Button {
                    store.beginComposeMode()
                    router.navigate(to: .compose, coordinator: coordinator)
                } label: {
                    Image(systemName: "square.and.pencil")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.localized("hig.common.compose"))
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 6)
    }

    // MARK: - Stories Entry Strip

    private var storiesTrayStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 12) {
                ForEach(Array(store.stories.enumerated()), id: \.element.id) { index, story in
                    Button {
                        store.overlayController.openStories(startingAt: index)
                    } label: {
                        VStack(spacing: 4) {
                            FabushiAvatarBadge(
                                peerId: Int64(bitPattern: story.userId),
                                title: story.firstName.isEmpty ? (story.username ?? "S") : story.firstName,
                                isOnline: false,
                                isBot: false
                            )
                            .padding(2)
                            .overlay(
                                Circle().strokeBorder(
                                    story.isViewed ? theme.separatorHairline : theme.accentColor,
                                    lineWidth: 2
                                )
                            )

                            Text(story.firstName.isEmpty ? (story.username ?? "") : story.firstName)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(1)
                                .frame(maxWidth: 56)
                        }
                    }
                    .buttonStyle(.plain)
                    .fabushiMinimumTouchTarget()
                    .accessibilityLabel("\(theme.messagingString(.storiesTitle)): \(story.firstName)")
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.storiesStrip)
    }

    // MARK: - Folder Pill Bar

    private var folderPillBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                folderChip(
                    title: theme.messagingString(.allFolder),
                    isSelected: !store.showArchivedSection && store.selectedCustomFolderId == nil && store.selectedBuiltInFolder == .all,
                    accessibilityID: FabushiAccessibilityIDs.ChatList.folderAllTab
                ) {
                    store.showArchivedSection = false
                    store.selectedCustomFolderId = nil
                    store.selectedBuiltInFolder = .all
                }

                folderChip(
                    title: theme.messagingString(.unreadFolder),
                    isSelected: !store.showArchivedSection && store.selectedCustomFolderId == nil && store.selectedBuiltInFolder == .unread,
                    accessibilityID: FabushiAccessibilityIDs.ChatList.folderUnreadTab
                ) {
                    store.showArchivedSection = false
                    store.selectedCustomFolderId = nil
                    store.selectedBuiltInFolder = .unread
                }

                folderChip(
                    title: theme.messagingString(.botsFolder),
                    isSelected: !store.showArchivedSection && store.selectedCustomFolderId == nil && store.selectedBuiltInFolder == .bots,
                    accessibilityID: FabushiAccessibilityIDs.ChatList.folderBotsTab
                ) {
                    store.showArchivedSection = false
                    store.selectedCustomFolderId = nil
                    store.selectedBuiltInFolder = .bots
                }

                ForEach(store.folders) { folder in
                    let label = [folder.emoticon, folder.title].compactMap { $0 }.joined(separator: " ")
                    folderChip(
                        title: label,
                        isSelected: !store.showArchivedSection && store.selectedCustomFolderId == folder.id,
                        accessibilityID: "fabushi.chatlist.folder.custom.\(folder.id)"
                    ) {
                        store.showArchivedSection = false
                        store.selectedCustomFolderId = folder.id
                    }
                    .contextMenu {
                        Button {
                            editingFolder = folder
                            isFolderEditorPresented = true
                        } label: {
                            Label(theme.messagingString(.editFolder), systemImage: "pencil")
                        }
                        Button(role: .destructive) {
                            Task { @MainActor in
                                try? await store.deleteFolder(folderId: folder.id)
                            }
                        } label: {
                            Label(theme.messagingString(.deleteFolder), systemImage: "trash")
                        }
                    }
                }

                Button {
                    editingFolder = nil
                    isFolderEditorPresented = true
                } label: {
                    Image(systemName: "folder.badge.plus")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.accentColor)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(theme.surfaceInput, in: Capsule())
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.messagingString(.newFolder))
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.folderBar)
    }

    private func folderChip(
        title: String,
        isSelected: Bool,
        accessibilityID: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
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
        .accessibilityIdentifier(accessibilityID)
    }

    // MARK: - Outbox / Sync / Offline Preserved Status Banners

    @ViewBuilder
    private var statusBannersStack: some View {
        let visibleOutbox = store.visibleOutboxEntries
        let failedCount = visibleOutbox.filter { $0.status == .failed }.count
        let pendingCount = visibleOutbox.count

        if store.offlinePreservedMode {
            HStack(spacing: 8) {
                Image(systemName: "wifi.slash")
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.warningColor)
                    .accessibilityHidden(true)
                Text(theme.messagingString(.offlinePreservedBanner))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(theme.surfaceSecondary)
        }

        if pendingCount > 0 {
            HStack(spacing: 8) {
                Image(systemName: failedCount > 0 ? "exclamationmark.triangle.fill" : "clock.arrow.circlepath")
                    .fabushiFont(.caption)
                    .foregroundStyle(failedCount > 0 ? theme.destructiveColor : theme.accentColor)
                    .accessibilityHidden(true)

                Text(
                    failedCount > 0
                        ? theme.messagingFormat(.outboxFailedBannerFormat, failedCount)
                        : theme.messagingFormat(.outboxPendingBannerFormat, pendingCount)
                )
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)
                .frame(maxWidth: .infinity, alignment: .leading)

                Button {
                    Task { @MainActor in
                        _ = await store.retryAllOutbox()
                    }
                } label: {
                    Text(theme.messagingString(.outboxRetryAllAction))
                        .fabushiFont(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.accentColor)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(theme.surfaceInput, in: Capsule())
                }
                .buttonStyle(.plain)
                .fabushiMinimumTouchTarget()
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(theme.surfaceSecondary)
        } else if store.isRecoveringSyncGaps {
            HStack(spacing: 8) {
                ProgressView()
                    .controlSize(.small)
                Text(theme.messagingString(.syncGapRecovering))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(theme.surfaceSecondary)
        }
    }

    // MARK: - Main List / Skeleton / Empty State (`D13-03`)

    @ViewBuilder
    private var mainContentArea: some View {
        if shouldShowLoadingSkeleton {
            FabushiDialogListSkeletonView()
        } else if displayedChats.isEmpty && (store.showArchivedSection || store.archivedChatsCount == 0) {
            FabushiEmptyStateView(
                title: theme.localized("hig.empty.chatsTitle"),
                systemImage: "bubble.left.and.bubble.right",
                descriptionText: theme.localized("hig.empty.chatsDescription"),
                accessibilityID: FabushiAccessibilityIDs.ChatList.emptyState
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            List {
                if !store.showArchivedSection && store.archivedChatsCount > 0 && store.selectedCustomFolderId == nil && store.selectedBuiltInFolder == .all {
                    Button {
                        store.showArchivedSection = true
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "archivebox.fill")
                                .fabushiFont(.title3)
                                .foregroundStyle(theme.textSecondary)
                                .frame(width: 44, height: 44)
                                .background(theme.surfaceInput, in: Circle())
                                .accessibilityHidden(true)

                            VStack(alignment: .leading, spacing: 2) {
                                Text(theme.messagingString(.archivedChats))
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.textPrimary)
                                Text("\(store.archivedChatsCount)")
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                            Spacer()
                            Image(systemName: "chevron.right")
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textMuted)
                                .accessibilityHidden(true)
                        }
                        .padding(.vertical, 4)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(theme.surfacePrimary)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.archiveRow)
                }

                if store.showArchivedSection {
                    Button {
                        store.showArchivedSection = false
                    } label: {
                        Label(theme.localized("hig.common.back"), systemImage: "chevron.left")
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.accentColor)
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(theme.surfacePrimary)
                }

                ForEach(Array(displayedChats.enumerated()), id: \.element.id) { index, chat in
                    let isSelected = store.activeChatId == chat.chatId
                    Button {
                        if chat.isForum {
                            activeTopicsChat = chat
                        } else {
                            openChat(chat)
                        }
                    } label: {
                        FabushiChatRowContentView(
                            chat: chat,
                            isSelected: isSelected,
                            draftText: store.unsentDraftTextByChat[chat.chatId] ?? chat.draft?.text,
                            botActivity: store.canDisplayBotActivity(for: chat.chatId)
                                ? store.activeBotActivityByChat[chat.chatId]
                                : nil,
                            isTyping: !(store.typingUsersByChat[chat.chatId]?.isEmpty ?? true)
                        )
                    }
                    .buttonStyle(.plain)
                    .listRowInsets(EdgeInsets(top: 6, leading: 12, bottom: 6, trailing: 12))
                    .listRowBackground(isSelected ? theme.accentColor.opacity(0.12) : theme.surfacePrimary)
                    .accessibilityIdentifier(rowAccessibilityID(for: index))
                    .swipeActions(edge: .leading, allowsFullSwipe: true) {
                        Button {
                            Task { @MainActor in
                                await store.togglePin(chatId: chat.chatId)
                            }
                        } label: {
                            Label(
                                chat.isPinned
                                    ? theme.messagingString(.unpinAction)
                                    : theme.messagingString(.pinAction),
                                systemImage: chat.isPinned ? "pin.slash" : "pin"
                            )
                        }
                        .tint(theme.accentColor)

                        if chat.unreadCount > 0 {
                            Button {
                                Task { @MainActor in
                                    await store.markChatRead(chatId: chat.chatId)
                                }
                            } label: {
                                Label(theme.messagingString(.markReadAction), systemImage: "envelope.open")
                            }
                            .tint(theme.successColor)
                        }
                    }
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button(role: .destructive) {
                            pendingDeleteChat = chat
                        } label: {
                            Label(theme.messagingString(.deleteChatAction), systemImage: "trash")
                        }

                        Button {
                            Task { @MainActor in
                                await store.toggleArchive(chatId: chat.chatId)
                            }
                        } label: {
                            Label(
                                chat.isArchived
                                    ? theme.messagingString(.unarchiveAction)
                                    : theme.messagingString(.archiveAction),
                                systemImage: "archivebox"
                            )
                        }
                        .tint(theme.textSecondary)

                        Button {
                            Task { @MainActor in
                                await store.toggleMute(chatId: chat.chatId)
                            }
                        } label: {
                            Label(
                                chat.isMuted
                                    ? theme.messagingString(.unmuteAction)
                                    : theme.messagingString(.muteAction),
                                systemImage: chat.isMuted ? "bell" : "bell.slash"
                            )
                        }
                        .tint(theme.warningColor)
                    }
                    .contextMenu {
                        Button {
                            Task { @MainActor in await store.togglePin(chatId: chat.chatId) }
                        } label: {
                            Label(
                                chat.isPinned ? theme.messagingString(.unpinAction) : theme.messagingString(.pinAction),
                                systemImage: chat.isPinned ? "pin.slash" : "pin"
                            )
                        }
                        Button {
                            Task { @MainActor in await store.toggleMute(chatId: chat.chatId) }
                        } label: {
                            Label(
                                chat.isMuted ? theme.messagingString(.unmuteAction) : theme.messagingString(.muteAction),
                                systemImage: chat.isMuted ? "bell" : "bell.slash"
                            )
                        }
                        Button {
                            Task { @MainActor in await store.toggleArchive(chatId: chat.chatId) }
                        } label: {
                            Label(
                                chat.isArchived ? theme.messagingString(.unarchiveAction) : theme.messagingString(.archiveAction),
                                systemImage: "archivebox"
                            )
                        }
                        if chat.unreadCount > 0 {
                            Button {
                                Task { @MainActor in await store.markChatRead(chatId: chat.chatId) }
                            } label: {
                                Label(theme.messagingString(.markReadAction), systemImage: "envelope.open")
                            }
                        }
                        if chat.isForum {
                            Button {
                                activeTopicsChat = chat
                            } label: {
                                Label(theme.messagingString(.forumTopicsTitle), systemImage: "list.bullet.indent")
                            }
                        }
                        Divider()
                        Button(role: .destructive) {
                            pendingDeleteChat = chat
                        } label: {
                            Label(theme.messagingString(.deleteChatAction), systemImage: "trash")
                        }
                    }
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .refreshable {
                await store.refreshWorkspace(trigger: .pullToRefresh)
            }
            .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.list)
        }
    }

    private func openChat(_ chat: FabushiMessagingChat) {
        if store.isComposeActive {
            store.cancelComposeMode()
        }
        router.navigate(to: .chats(chatId: chat.chatId, messageId: nil), coordinator: coordinator)
        Task { @MainActor in
            await store.selectChat(chat.chatId)
        }
    }

    private func rowAccessibilityID(for index: Int) -> String {
        switch index {
        case 0: return FabushiAccessibilityIDs.ChatList.firstRow
        case 1: return FabushiAccessibilityIDs.ChatList.secondRow
        case 2: return FabushiAccessibilityIDs.ChatList.thirdRow
        default: return "fabushi.chatlist.row.\(index)"
        }
    }
}

// MARK: - Chat List Row Content

@MainActor
private struct FabushiChatRowContentView: View {
    let chat: FabushiMessagingChat
    let isSelected: Bool
    let draftText: String?
    let botActivity: FabushiBotActivityState?
    let isTyping: Bool

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var formattedTime: String {
        guard chat.lastMessageDateMs > 0 else { return "" }
        let date = Date(timeIntervalSince1970: TimeInterval(chat.lastMessageDateMs) / 1000.0)
        if Calendar.current.isDateInToday(date) {
            return date.formatted(date: .omitted, time: .shortened)
        }
        return date.formatted(.dateTime.month(.abbreviated).day())
    }

    var body: some View {
        HStack(spacing: 12) {
            FabushiAvatarBadge(
                peerId: chat.chatId,
                title: chat.title.isEmpty ? theme.messagingString(.personalNotes) : chat.title,
                isOnline: false,
                isBot: chat.isBot && !chat.isGroup && !chat.isChannel
            )

            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(chat.title.isEmpty ? theme.messagingString(.personalNotes) : chat.title)
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)

                    if chat.isForum {
                        Image(systemName: "number.square")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textSecondary)
                            .accessibilityHidden(true)
                    } else if chat.isChannel {
                        Image(systemName: "megaphone")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textSecondary)
                            .accessibilityHidden(true)
                    } else if chat.isGroup {
                        Image(systemName: "person.2.fill")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textSecondary)
                            .accessibilityHidden(true)
                    }

                    if chat.isMuted {
                        Image(systemName: "bell.slash.fill")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                            .accessibilityLabel(theme.messagingString(.mutedBadge))
                    }

                    Spacer(minLength: 4)

                    if !formattedTime.isEmpty {
                        Text(formattedTime)
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                    }
                }

                HStack(alignment: .center, spacing: 6) {
                    subtitleView
                        .frame(maxWidth: .infinity, alignment: .leading)

                    if chat.isPinned {
                        Image(systemName: "pin.fill")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                            .accessibilityLabel(theme.messagingString(.pinnedBadge))
                    }

                    if chat.unreadCount > 0 {
                        Text(chat.unreadCount > 99 ? "99+" : "\(chat.unreadCount)")
                            .fabushiFont(.caption2)
                            .fontWeight(.semibold)
                            .foregroundStyle(theme.accentForegroundColor)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 2)
                            .background(
                                chat.isMuted ? theme.textMuted : theme.accentColor,
                                in: Capsule()
                            )
                            .accessibilityLabel(theme.messagingFormat(.unreadCountFormat, chat.unreadCount))
                            .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.unreadBadge)
                    }
                }
            }
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private var subtitleView: some View {
        if let botActivity, botActivity.isActive {
            HStack(spacing: 4) {
                Image(systemName: "sparkles")
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)
                Text(botActivity.detail ?? theme.messagingString(.botThinkingIndicator))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.accentColor)
                    .lineLimit(1)
            }
        } else if isTyping {
            Text(theme.messagingString(.typingIndicator))
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.accentColor)
                .lineLimit(1)
        } else if let draft = draftText?.trimmingCharacters(in: .whitespacesAndNewlines), !draft.isEmpty {
            HStack(spacing: 4) {
                Text(theme.messagingString(.draftPrefix))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.destructiveColor)
                Text(draft)
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
                    .lineLimit(1)
            }
        } else {
            Text(chat.lastMessagePreview ?? "")
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.textSecondary)
                .lineLimit(1)
        }
    }
}

// MARK: - Folder Editor Sheet

@MainActor
private struct FabushiFolderEditorSheet: View {
    let existingFolder: FabushiFolder?
    let onSave: (FabushiFolder) -> Void
    let onDelete: (Int64) -> Void
    let onCancel: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var title: String
    @State private var includeGroups: Bool
    @State private var includeBots: Bool
    @State private var includeChannels: Bool
    @State private var excludeMuted: Bool
    @State private var excludeRead: Bool
    @State private var excludeArchived: Bool

    private var theme: FabushiTheme { injectedTheme ?? .default }

    init(
        existingFolder: FabushiFolder?,
        onSave: @escaping (FabushiFolder) -> Void,
        onDelete: @escaping (Int64) -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.existingFolder = existingFolder
        self.onSave = onSave
        self.onDelete = onDelete
        self.onCancel = onCancel
        _title = State(initialValue: existingFolder?.title ?? "")
        _includeGroups = State(initialValue: existingFolder?.includeGroups ?? true)
        _includeBots = State(initialValue: existingFolder?.includeBots ?? false)
        _includeChannels = State(initialValue: existingFolder?.includeChannels ?? false)
        _excludeMuted = State(initialValue: existingFolder?.excludeMuted ?? false)
        _excludeRead = State(initialValue: existingFolder?.excludeRead ?? false)
        _excludeArchived = State(initialValue: existingFolder?.excludeArchived ?? true)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(theme.messagingString(.folderNamePlaceholder), text: $title)
                        .fabushiFont(.body)
                }

                Section {
                    Toggle(theme.messagingString(.includeGroups), isOn: $includeGroups)
                    Toggle(theme.messagingString(.includeBots), isOn: $includeBots)
                    Toggle(theme.messagingString(.includeChannels), isOn: $includeChannels)
                    Toggle(theme.messagingString(.excludeMuted), isOn: $excludeMuted)
                    Toggle(theme.messagingString(.excludeRead), isOn: $excludeRead)
                    Toggle(theme.messagingString(.excludeArchived), isOn: $excludeArchived)
                }

                if let existingFolder, existingFolder.id > 0 {
                    Section {
                        Button(role: .destructive) {
                            onDelete(existingFolder.id)
                        } label: {
                            Text(theme.messagingString(.deleteFolder))
                                .fabushiFont(.body)
                        }
                    }
                }
            }
            .navigationTitle(
                existingFolder == nil
                    ? theme.messagingString(.newFolder)
                    : theme.messagingString(.editFolder)
            )
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.localized("hig.common.done")) {
                        let folder = FabushiFolder(
                            id: existingFolder?.id ?? 0,
                            title: title.trimmingCharacters(in: .whitespacesAndNewlines),
                            emoticon: existingFolder?.emoticon,
                            includeChats: existingFolder?.includeChats ?? [],
                            excludeChats: existingFolder?.excludeChats ?? [],
                            pinnedChats: existingFolder?.pinnedChats ?? [],
                            includeContacts: existingFolder?.includeContacts ?? false,
                            includeNonContacts: existingFolder?.includeNonContacts ?? false,
                            includeGroups: includeGroups,
                            includeChannels: includeChannels,
                            includeBots: includeBots,
                            excludeMuted: excludeMuted,
                            excludeRead: excludeRead,
                            excludeArchived: excludeArchived,
                            sortOrder: existingFolder?.sortOrder ?? 0
                        )
                        onSave(folder)
                    }
                    .disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

// MARK: - Forum Topics Sheet (`topics.*`)

@MainActor
private struct FabushiForumTopicsSheetView: View {
    let chat: FabushiMessagingChat
    let store: FabushiMessagingStore
    let onSelectTopic: (FabushiForumTopic) -> Void
    let onClose: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var newTopicTitle: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var topics: [FabushiForumTopic] {
        store.topicsByChat[chat.chatId] ?? []
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                TextField(theme.messagingString(.topicTitlePlaceholder), text: $newTopicTitle)
                    .fabushiFont(.body)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                Button {
                    let trimmed = newTopicTitle.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !trimmed.isEmpty else { return }
                    Task { @MainActor in
                        _ = try? await store.createTopic(chatId: chat.chatId, title: trimmed)
                        newTopicTitle = ""
                    }
                } label: {
                    Text(theme.messagingString(.newTopicAction))
                        .fabushiFont(.callout)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.accentForegroundColor)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(theme.accentColor, in: Capsule())
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(newTopicTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(12)

            Divider().overlay(theme.separatorHairline)

            if topics.isEmpty {
                FabushiEmptyStateView(
                    title: theme.messagingString(.noTopicsYet),
                    systemImage: "number.square",
                    descriptionText: chat.title,
                    accessibilityID: FabushiAccessibilityIDs.ChatList.topicsList
                )
            } else {
                List(topics) { topic in
                    Button {
                        onSelectTopic(topic)
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: topic.isClosed ? "lock.fill" : "number")
                                .fabushiFont(.body)
                                .foregroundStyle(theme.accentColor)
                                .accessibilityHidden(true)

                            VStack(alignment: .leading, spacing: 2) {
                                Text(topic.title)
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.textPrimary)
                                if topic.isClosed {
                                    Text(theme.messagingString(.topicClosedBadge))
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.textMuted)
                                }
                            }
                            Spacer()
                            if topic.isPinned {
                                Image(systemName: "pin.fill")
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.textMuted)
                            }
                            if topic.unreadCount > 0 {
                                Text("\(topic.unreadCount)")
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.accentForegroundColor)
                                    .padding(.horizontal, 7)
                                    .padding(.vertical, 2)
                                    .background(theme.accentColor, in: Capsule())
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .swipeActions(edge: .trailing) {
                        Button(role: .destructive) {
                            Task { @MainActor in
                                try? await store.deleteTopic(chatId: chat.chatId, topicId: topic.topicId)
                            }
                        } label: {
                            Label(theme.messagingString(.deleteChatAction), systemImage: "trash")
                        }
                        Button {
                            Task { @MainActor in
                                try? await store.updateTopic(
                                    chatId: chat.chatId,
                                    topicId: topic.topicId,
                                    isClosed: !topic.isClosed
                                )
                            }
                        } label: {
                            Label(theme.messagingString(.topicClosedBadge), systemImage: topic.isClosed ? "lock.open" : "lock")
                        }
                        .tint(theme.warningColor)
                    }
                }
                .listStyle(.plain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.topicsList)
            }
        }
        .navigationTitle("\(chat.title) · \(theme.messagingString(.forumTopicsTitle))")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button(theme.localized("hig.common.close"), action: onClose)
            }
        }
        .task {
            await store.loadTopics(chatId: chat.chatId)
        }
    }
}
