import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Canonical Conversation Stage View (`#12`, `#13`, `#14`, `D10-08`, `D13-03`, `D14-04`)

/// Bottom-anchored virtualized conversation stage supporting all 11+ canonical cards, 900s bubble
/// clustering, message actions, multi-select forward/delete, reactions, pins, TTL (`D10-08`),
/// outbox recovery, forum topics, E2EE badge, group+DM bot activity (`D14-04`), IME-safe composer,
/// and live overlay presentation (`FabushiMessagingOverlayController`).
///
/// Shell layout invariants:
/// - NEVER nests a second `NavigationStack` (owned by `FabushiAdaptiveShellView`).
/// - Keeps toolbar buttons strictly in `.topBarTrailing` so compact iPhone `.topBarLeading`
///   and `.principal` shell controls never collide.
@MainActor
public struct FabushiConversationStageView: View {
    public let chatId: Int64?
    public let highlightedMessageId: Int64?
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(FabushiFeatureModuleRegistry.self) private var injectedRegistry: FabushiFeatureModuleRegistry?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var voicePlayer = FabushiVoicePlayerController()
    @State private var isMultiSelectMode: Bool = false
    @State private var selectedMessageIds: Set<Int64> = []
    @State private var isJumpToBottomVisible: Bool = false
    @State private var isPinsSheetPresented: Bool = false
    @State private var isForwardPickerPresented: Bool = false
    @State private var forwardSourceMessageIds: [Int64] = []
    @State private var isTTLPickerPresented: Bool = false
    @State private var isClearHistoryConfirmPresented: Bool = false
    @State private var isLeaveConfirmPresented: Bool = false
    @State private var isStatisticsSheetPresented: Bool = false
    @State private var loadedStatistics: FabushiChannelStatistics? = nil
    @State private var exportedJSON: String? = nil
    @State private var prestartProfile: FabushiBotPublicProfile? = nil

    private static let quickReactionEmojis: [String] = ["❤️", "👍", "🔥", "😂", "😮", "😢", "🙏"]

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var activeChat: FabushiMessagingChat? {
        guard let chatId else { return nil }
        return store.chats.first(where: { $0.chatId == chatId })
    }

    private var stageMessages: [FabushiMessageRecord] {
        guard let chatId else { return [] }
        let all = store.messagesByChat[chatId] ?? []
        if let activeTopicId = store.activeTopicIdByChat[chatId] {
            return all.filter { $0.topicId == nil || $0.topicId == activeTopicId }
        }
        return all
    }

    private var clusteredMessages: [FabushiClusteredMessageItem] {
        FabushiMessageClusterCalculator.compute(
            messages: stageMessages,
            isGroupChat: activeChat?.isGroup == true || activeChat?.isChannel == true,
            locale: theme.locale.foundationLocale
        )
    }

    private var chatOutboxEntries: [FabushiOutboxEntry] {
        guard let chatId else { return [] }
        return store.visibleOutboxEntries.filter { $0.chatId == chatId }
    }

    private var pinnedMessages: [FabushiPinnedMessageRecord] {
        guard let chatId else { return [] }
        return store.pinnedMessagesByChat[chatId] ?? []
    }

    private var forumTopics: [FabushiForumTopic] {
        guard let chatId else { return [] }
        return store.topicsByChat[chatId] ?? []
    }

    private var distilledScripts: [FabushiDistilledBrowserScript] {
        guard let chatId else { return [] }
        return store.distilledScriptsByChat[chatId] ?? []
    }

    private var shouldShowBotActivityBubble: Bool {
        guard let chatId,
              let activity = store.activeBotActivityByChat[chatId],
              activity.isActive
        else {
            return false
        }
        // `D14-04`: Render in BOTH 1:1 bot DMs AND group chats!
        return store.canDisplayBotActivity(for: chatId)
    }

    public init(
        chatId: Int64?,
        messageId: Int64? = nil,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil
    ) {
        self.chatId = chatId
        self.highlightedMessageId = messageId
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
    }

    public var body: some View {
        Group {
            if let chatId {
                activeConversationBody(chatId: chatId)
            } else if !store.hasSyncedDialogs && store.chats.isEmpty {
                // `D13-03`: iPad empty detail syncing pill while initial dialog sync is in flight
                FabushiStageSyncingPlaceholderView()
            } else {
                FabushiEmptyStateView(
                    title: theme.messagingString(.chatsTitle),
                    systemImage: "bubble.left.and.bubble.right",
                    descriptionText: theme.localized("hig.empty.selectChat"),
                    accessibilityID: "fabushi.stage.emptySelection"
                )
            }
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.root)
        // Live Overlay Sheets bridged via `store.overlayController` (works live without editing frozen shell)
        .sheet(isPresented: Binding(
            get: { store.overlayController.isUniversalSearchPresented },
            set: { if !$0 { store.overlayController.dismissUniversalSearch() } }
        )) {
            NavigationStack {
                FabushiUniversalSearchView(
                    coordinator: coordinator,
                    router: router,
                    store: store,
                    inChatId: store.overlayController.activeSearchChatId ?? chatId,
                    onDismiss: { store.overlayController.dismissUniversalSearch() }
                )
            }
        }
        .fullScreenCover(item: Binding(
            get: { store.overlayController.presentedMediaTarget },
            set: { if $0 == nil { store.overlayController.dismissMediaLightbox() } }
        )) { target in
            FabushiMediaLightboxView(
                coordinator: coordinator,
                router: router,
                store: store,
                target: target,
                onDismiss: { store.overlayController.dismissMediaLightbox() }
            )
        }
        .fullScreenCover(isPresented: Binding(
            get: { store.overlayController.isStoriesOverlayPresented },
            set: { if !$0 { store.overlayController.dismissStories() } }
        )) {
            FabushiStoriesView(
                coordinator: coordinator,
                router: router,
                store: store,
                onDismiss: { store.overlayController.dismissStories() }
            )
        }
        .sheet(item: Binding(
            get: { store.overlayController.presentedInstantView },
            set: { if $0 == nil { store.overlayController.dismissInstantView() } }
        )) { doc in
            FabushiInstantViewReaderView(
                coordinator: coordinator,
                router: router,
                store: store,
                document: doc,
                onDismiss: { store.overlayController.dismissInstantView() }
            )
        }
        .sheet(item: Binding(
            get: { store.overlayController.presentedSafetyInfo },
            set: { if $0 == nil { store.overlayController.dismissE2EEVerification() } }
        )) { safetyInfo in
            FabushiE2EESafetyView(
                chatId: safetyInfo.chatId,
                peerTitle: safetyInfo.peerTitle,
                remoteIdentityKeyB64: safetyInfo.remoteIdentityKeyB64,
                store: store,
                router: router,
                onDismiss: { store.overlayController.dismissE2EEVerification() }
            )
        }
    }

    // MARK: - Active Conversation Stage

    private func activeConversationBody(chatId: Int64) -> some View {
        VStack(spacing: 0) {
            stageInlineHeaderBar(chatId: chatId)

            if !pinnedMessages.isEmpty {
                pinnedMessageBanner(chatId: chatId)
            }

            if activeChat?.isForum == true && !forumTopics.isEmpty {
                forumTopicsStrip(chatId: chatId)
            }

            if let speaker = store.floorSpeakerByChat[chatId], !speaker.isEmpty {
                floorControlBanner(chatId: chatId, speaker: speaker)
            }

            if !chatOutboxEntries.isEmpty {
                outboxRecoveryBanner(chatId: chatId)
            }

            // Cross-module M4 Agent Studio accessory slots (safe EmptyView when M4 not yet registered)
            if let registry = injectedRegistry {
                registry.agentStudio.makeCodingAgentThreadAccessories(
                    chatId: chatId,
                    coordinator: coordinator,
                    router: router
                )
                registry.agentStudio.makeMultiAgentRoomAccessories(
                    chatId: chatId,
                    coordinator: coordinator,
                    router: router
                )
            }

            // Prestart Bot Hero OR Virtualized Transcript
            if let prestart = prestartProfile, !prestart.hasStartedChat && stageMessages.isEmpty {
                FabushiPrestartBotHeroView(
                    profile: prestart,
                    store: store,
                    coordinator: coordinator,
                    router: router,
                    onStarted: { _ in
                        prestartProfile = nil
                    }
                )
            } else {
                transcriptScrollArea(chatId: chatId)

                if isMultiSelectMode {
                    multiSelectToolbar(chatId: chatId)
                } else {
                    FabushiComposerBarView(
                        chatId: chatId,
                        chat: activeChat,
                        coordinator: coordinator,
                        router: router,
                        store: store
                    )
                }
            }
        }
        .background {
            theme.wallpaper.gradient.ignoresSafeArea()
        }
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    store.overlayController.openUniversalSearch(inChatId: chatId)
                } label: {
                    Image(systemName: "magnifyingglass")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.localized("hig.common.search"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.searchBar)

                Button {
                    router.navigate(to: .calls, coordinator: coordinator)
                } label: {
                    Image(systemName: "phone")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.messagingString(.inspectorActionCall))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.callStartButton)

                Button {
                    openInspectorForCurrentChat(chatId: chatId)
                } label: {
                    Image(systemName: "info.circle")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.localized("hig.inspector.title"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.inspectorToggleButton)
            }
        }
        .task(id: chatId) {
            isMultiSelectMode = false
            selectedMessageIds.removeAll()
            prestartProfile = nil
            store.startEventLoopIfNeeded()
            await store.selectChat(chatId)
            await store.refreshApprovals()
            if activeChat?.isForum == true {
                await store.loadTopics(chatId: chatId)
            }
            if activeChat?.isBot == true,
               (store.messagesByChat[chatId]?.isEmpty ?? true) {
                let target = activeChat?.username ?? String(chatId)
                prestartProfile = await store.loadBotPublicProfile(target: target)
            }
        }
        .onDisappear {
            voicePlayer.stop()
        }
        .sheet(isPresented: $isPinsSheetPresented) {
            pinnedMessagesSheet(chatId: chatId)
        }
        .sheet(isPresented: $isForwardPickerPresented) {
            forwardTargetPickerSheet(fromChatId: chatId)
        }
        .sheet(isPresented: $isTTLPickerPresented) {
            ttlPickerSheet(chatId: chatId)
        }
        .sheet(isPresented: $isStatisticsSheetPresented) {
            FabushiInspectorStatisticsSheet(
                title: activeChat?.title ?? "",
                stats: loadedStatistics,
                onClose: { isStatisticsSheetPresented = false }
            )
        }
        #if canImport(UIKit)
        .sheet(isPresented: Binding(
            get: { exportedJSON != nil },
            set: { if !$0 { exportedJSON = nil } }
        )) {
            if let exportedJSON {
                FabushiActivityShareSheet(items: [exportedJSON])
            }
        }
        #endif
        .confirmationDialog(
            theme.messagingString(.clearHistoryConfirmTitle),
            isPresented: $isClearHistoryConfirmPresented,
            titleVisibility: .visible
        ) {
            Button(theme.messagingString(.clearHistoryAction), role: .destructive) {
                Task { @MainActor in
                    _ = await store.clearChatHistory(
                        chatId: chatId,
                        router: router,
                        isCompact: horizontalSizeClass == .compact,
                        locale: theme.locale
                    )
                }
            }
            Button(theme.localized("hig.common.cancel"), role: .cancel) {}
        } message: {
            Text(theme.messagingString(.clearHistoryConfirmMessage))
        }
        .confirmationDialog(
            theme.messagingString(.inspectorLeaveConfirmTitle),
            isPresented: $isLeaveConfirmPresented,
            titleVisibility: .visible
        ) {
            Button(theme.messagingString(.leaveGroupAction), role: .destructive) {
                Task { @MainActor in
                    _ = await store.leaveChat(
                        chatId: chatId,
                        router: router,
                        locale: theme.locale
                    )
                }
            }
            Button(theme.localized("hig.common.cancel"), role: .cancel) {}
        } message: {
            Text(theme.messagingString(.inspectorLeaveConfirmMessage))
        }
    }

    // MARK: - Inline Stage Sub-Header (Title, Activity Subtitle, E2EE Badge, Actions Menu)

    private func stageInlineHeaderBar(chatId: Int64) -> some View {
        let title = activeChat?.title.isEmpty == false ? activeChat!.title : "Chat \(chatId)"
        let isVerifiedE2EE = store.verifiedE2EEChatIds.contains(chatId)

        return HStack(spacing: 10) {
            Button {
                openInspectorForCurrentChat(chatId: chatId)
            } label: {
                HStack(spacing: 10) {
                    FabushiAvatarBadge(
                        peerId: chatId,
                        title: title,
                        isOnline: false,
                        isBot: activeChat?.isBot == true
                    )
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(title)
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                                .lineLimit(1)
                                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.headerTitle)

                            if isVerifiedE2EE {
                                Image(systemName: "lock.shield.fill")
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.successColor)
                                    .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.e2eeVerifiedBadge)
                            }
                        }

                        if let subtitle = stageStatusSubtitle(chatId: chatId) {
                            Text(subtitle)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.accentColor)
                                .lineLimit(1)
                        }
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Spacer(minLength: 6)

            Button {
                Task { @MainActor in
                    if let info = try? await store.loadE2EESafetyInfo(chatId: chatId, peerTitle: title) {
                        store.overlayController.openE2EEVerification(info)
                    }
                }
            } label: {
                Image(systemName: isVerifiedE2EE ? "checkmark.shield.fill" : "lock.shield")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(isVerifiedE2EE ? theme.successColor : theme.textSecondary)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.messagingString(.e2eeSafetyNumbersTitle))

            Menu {
                Button {
                    isTTLPickerPresented = true
                } label: {
                    Label(theme.messagingString(.ttlTitle), systemImage: "timer")
                }

                Button {
                    Task { @MainActor in
                        loadedStatistics = try? await store.client.getChannelStatistics(chatId: chatId)
                        isStatisticsSheetPresented = true
                    }
                } label: {
                    Label(theme.messagingString(.inspectorStatisticsAction), systemImage: "chart.bar")
                }

                Button {
                    Task { @MainActor in
                        do {
                            exportedJSON = try await store.exportChatHistoryJSON(chatId: chatId)
                            _ = router.showChatToastIfVisible(
                                theme.messagingString(.exportHistoryDoneToast),
                                chatId: chatId,
                                coordinator: coordinator,
                                isCompact: horizontalSizeClass == .compact,
                                isError: false
                            )
                        } catch {
                            _ = router.showChatToastIfVisible(
                                theme.messagingString(.exportHistoryFailedToast),
                                chatId: chatId,
                                coordinator: coordinator,
                                isCompact: horizontalSizeClass == .compact,
                                isError: true
                            )
                        }
                    }
                } label: {
                    Label(theme.messagingString(.inspectorExportChatAction), systemImage: "square.and.arrow.up")
                }

                Button {
                    isMultiSelectMode.toggle()
                    if !isMultiSelectMode { selectedMessageIds.removeAll() }
                } label: {
                    Label(theme.messagingString(.messageSelectAction), systemImage: "checkmark.circle")
                }

                Divider()

                Button(role: .destructive) {
                    isClearHistoryConfirmPresented = true
                } label: {
                    Label(theme.messagingString(.clearHistoryAction), systemImage: "trash")
                }

                if activeChat?.isGroup == true || activeChat?.isChannel == true {
                    Button(role: .destructive) {
                        isLeaveConfirmPresented = true
                    } label: {
                        Label(theme.messagingString(.leaveGroupAction), systemImage: "rectangle.portrait.and.arrow.right")
                    }
                }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
                    .fabushiMinimumTouchTarget()
            }
            .accessibilityLabel(theme.messagingString(.inspectorActionMore))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(theme.surfaceTopBar)
        .overlay(alignment: .bottom) {
            Divider().overlay(theme.separatorHairline)
        }
    }

    private func stageStatusSubtitle(chatId: Int64) -> String? {
        if shouldShowBotActivityBubble, let activity = store.activeBotActivityByChat[chatId] {
            if let detail = activity.detail, !detail.isEmpty {
                return detail
            }
            return activity.state.lowercased().contains("respond")
                ? theme.messagingString(.botRespondingIndicator)
                : theme.messagingString(.botThinkingIndicator)
        }
        if let typing = store.typingUsersByChat[chatId], !typing.isEmpty {
            return theme.messagingString(.typingIndicator)
        }
        if let chat = activeChat, (chat.isGroup || chat.isChannel), chat.memberCount > 0 {
            return "\(chat.memberCount) \(theme.messagingString(.inspectorMembersTab))"
        }
        return nil
    }

    // MARK: - Banners (Pinned, Forum Topics, Floor Control, Outbox)

    private func pinnedMessageBanner(chatId: Int64) -> some View {
        let latestPin = pinnedMessages.first
        return Button {
            isPinsSheetPresented = true
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "pin.fill")
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.accentColor)
                VStack(alignment: .leading, spacing: 1) {
                    Text(theme.messagingString(.pinnedMessagesSheetTitle))
                        .fabushiFont(.caption2)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.accentColor)
                    Text(latestPin?.previewText.isEmpty == false ? latestPin!.previewText : "#\(latestPin?.messageId ?? 0)")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)
                }
                Spacer()
                Text("\(pinnedMessages.count)")
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.textMuted)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(theme.surfaceElevated)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.pinnedBanner)
    }

    private func forumTopicsStrip(chatId: Int64) -> some View {
        let activeTopicId = store.activeTopicIdByChat[chatId]
        return ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                Button {
                    store.activeTopicIdByChat.removeValue(forKey: chatId)
                } label: {
                    Text(theme.messagingString(.allFolder))
                        .fabushiFont(.caption)
                        .foregroundStyle(activeTopicId == nil ? theme.accentForegroundColor : theme.textSecondary)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(activeTopicId == nil ? theme.accentColor : theme.surfaceInput, in: Capsule())
                }
                .buttonStyle(.plain)

                ForEach(forumTopics) { topic in
                    let isSelected = activeTopicId == topic.topicId
                    Button {
                        store.activeTopicIdByChat[chatId] = topic.topicId
                    } label: {
                        Text(topic.title)
                            .fabushiFont(.caption)
                            .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(isSelected ? theme.accentColor : theme.surfaceInput, in: Capsule())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
        .background(theme.surfaceSecondary)
    }

    private func floorControlBanner(chatId: Int64, speaker: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "waveform.circle.fill")
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.accentColor)
            Text(speaker)
                .fabushiFont(.caption)
                .fontWeight(.semibold)
                .foregroundStyle(theme.textPrimary)
            Spacer()
            Button {
                Task { @MainActor in
                    await store.preemptFloor(chatId: chatId)
                }
            } label: {
                Text(theme.messagingString(.floorInterruptAction))
                    .fabushiFont(.caption2)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.accentForegroundColor)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(theme.destructiveColor, in: Capsule())
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.floorInterruptButton)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(theme.surfaceElevated)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.floorControlBanner)
    }

    private func outboxRecoveryBanner(chatId: Int64) -> some View {
        let failedCount = chatOutboxEntries.filter { $0.status == .failed }.count
        let text = failedCount > 0
            ? theme.messagingFormat(.outboxFailedBannerFormat, failedCount)
            : theme.messagingFormat(.outboxPendingBannerFormat, chatOutboxEntries.count)

        return HStack(spacing: 10) {
            Image(systemName: failedCount > 0 ? "exclamationmark.triangle.fill" : "clock.arrow.circlepath")
                .fabushiFont(.caption)
                .foregroundStyle(failedCount > 0 ? theme.destructiveColor : theme.warningColor)
            Text(text)
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)
            Spacer()
            Button {
                Task { @MainActor in
                    _ = await store.retryAllOutbox()
                }
            } label: {
                Text(theme.messagingString(.outboxRetryAllAction))
                    .fabushiFont(.caption2)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.accentColor)
            }
            .buttonStyle(.plain)

            if let first = chatOutboxEntries.first {
                Button {
                    store.dismissOutboxEntry(clientNonce: first.clientNonce)
                } label: {
                    Text(theme.messagingString(.outboxDismissItemAction))
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textMuted)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(theme.surfaceElevated)
    }

    // MARK: - Virtualized Transcript (`ScrollViewReader` + `LazyVStack`)

    private func transcriptScrollArea(chatId: Int64) -> some View {
        ZStack(alignment: .bottomTrailing) {
            if store.isLoadingHistoryByChat[chatId] == true && stageMessages.isEmpty {
                // `D13-03`: 6-bubble Reduce-Motion-safe skeleton while uncached history loads
                FabushiTranscriptSkeletonView()
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 4) {
                            ForEach(Array(clusteredMessages.enumerated()), id: \.element.id) { index, item in
                                let isLatest = index == clusteredMessages.count - 1
                                let isSelected = selectedMessageIds.contains(item.message.messageId)

                                FabushiMessageCardView(
                                    item: item,
                                    chatId: chatId,
                                    isLatest: isLatest,
                                    isSelectedInMultiSelect: isSelected,
                                    voicePlayer: voicePlayer,
                                    coordinator: coordinator,
                                    router: router,
                                    store: store,
                                    onReplyTap: {
                                        if let replyId = item.message.replyTo?.replyToMsgId {
                                            proxy.scrollTo(replyId, anchor: .center)
                                        }
                                    },
                                    onReactionTap: { emoji in
                                        Task { @MainActor in
                                            _ = await store.toggleReaction(
                                                chatId: chatId,
                                                messageId: item.message.messageId,
                                                emoji: emoji
                                            )
                                        }
                                    },
                                    onRetryFailedTap: {
                                        Task { @MainActor in
                                            _ = await store.retryAllOutbox()
                                        }
                                    }
                                )
                                .id(item.message.messageId)
                                .onTapGesture {
                                    if isMultiSelectMode {
                                        toggleMultiSelect(item.message.messageId)
                                    }
                                }
                                .contextMenu {
                                    messageContextMenuItems(for: item.message, chatId: chatId)
                                }
                            }

                            // Distilled Browser Script cards (`D11-01`)
                            ForEach(distilledScripts) { script in
                                FabushiDistilledScriptCardView(script: script) {
                                    router.navigate(to: .agentStudio(tab: "skills"), coordinator: coordinator)
                                }
                            }

                            // Pending Tool Permission Docks (`fabushi.approvalsList`)
                            ForEach(store.pendingApprovals) { approval in
                                FabushiToolPermissionDockView(
                                    approval: approval,
                                    onApprove: {
                                        Task { @MainActor in
                                            _ = await store.decideApproval(
                                                approvalId: approval.id,
                                                approved: true,
                                                router: router,
                                                chatId: chatId,
                                                isCompact: horizontalSizeClass == .compact,
                                                locale: theme.locale
                                            )
                                        }
                                    },
                                    onDeny: {
                                        Task { @MainActor in
                                            _ = await store.decideApproval(
                                                approvalId: approval.id,
                                                approved: false,
                                                router: router,
                                                chatId: chatId,
                                                isCompact: horizontalSizeClass == .compact,
                                                locale: theme.locale
                                            )
                                        }
                                    }
                                )
                            }

                            // `D14-04`: Bot Thinking Bubble in BOTH 1:1 Bot DMs AND Group Chats!
                            if shouldShowBotActivityBubble, let activity = store.activeBotActivityByChat[chatId] {
                                FabushiAgentThinkingBubbleView(activity: activity)
                            }

                            Color.clear
                                .frame(height: 1)
                                .id("fabushi-transcript-bottom-anchor")
                                .onAppear { isJumpToBottomVisible = false }
                                .onDisappear { isJumpToBottomVisible = true }
                        }
                        .padding(.vertical, 10)
                    }
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.transcriptList)
                    .onAppear {
                        if let highlighted = highlightedMessageId {
                            proxy.scrollTo(highlighted, anchor: .center)
                        } else {
                            proxy.scrollTo("fabushi-transcript-bottom-anchor", anchor: .bottom)
                        }
                    }
                    .onChange(of: stageMessages.last?.messageId) { _, _ in
                        if !isJumpToBottomVisible {
                            proxy.scrollTo("fabushi-transcript-bottom-anchor", anchor: .bottom)
                        }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if isJumpToBottomVisible {
                            Button {
                                proxy.scrollTo("fabushi-transcript-bottom-anchor", anchor: .bottom)
                                Task { @MainActor in
                                    await store.markChatRead(chatId: chatId)
                                }
                            } label: {
                                HStack(spacing: 4) {
                                    if let unread = activeChat?.unreadCount, unread > 0 {
                                        Text("\(unread)")
                                            .fabushiFont(.caption2)
                                            .fontWeight(.bold)
                                    }
                                    Image(systemName: "chevron.down")
                                        .fabushiFont(.subheadline)
                                }
                                .foregroundStyle(theme.accentForegroundColor)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 10)
                                .background(theme.accentColor, in: Capsule())
                                .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .padding(14)
                            .accessibilityLabel(theme.messagingString(.jumpToBottomAction))
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    // MARK: - Context Menu & Quick Reactions

    @ViewBuilder
    private func messageContextMenuItems(for message: FabushiMessageRecord, chatId: Int64) -> some View {
        // Quick emoji reaction bar
        Menu(theme.messagingString(.reactMessageActionLabel)) {
            ForEach(Self.quickReactionEmojis, id: \.self) { emoji in
                Button(emoji) {
                    Task { @MainActor in
                        _ = await store.toggleReaction(chatId: chatId, messageId: message.messageId, emoji: emoji)
                    }
                }
            }
        }

        Button {
            let senderName = activeChat?.title ?? "User \(message.senderId)"
            store.replyTargetByChat[chatId] = FabushiReplyTarget(
                messageId: message.messageId,
                senderName: senderName,
                quoteText: message.text
            )
        } label: {
            Label(theme.messagingString(.messageReplyAction), systemImage: "arrowshape.turn.up.left")
        }

        if message.isOutgoing && !message.isDeletedTombstone {
            Button {
                store.editingMessageByChat[chatId] = message
            } label: {
                Label(theme.messagingString(.messageEditAction), systemImage: "pencil")
            }
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.editMessageAction)
        }

        Button {
            #if canImport(UIKit)
            UIPasteboard.general.string = message.text
            #endif
            router.toastController.showStagePill(
                theme.messagingString(.messageCopiedPill),
                chatId: chatId
            )
        } label: {
            Label(theme.messagingString(.messageCopyAction), systemImage: "doc.on.doc")
        }

        Button {
            forwardSourceMessageIds = [message.messageId]
            isForwardPickerPresented = true
        } label: {
            Label(theme.messagingString(.messageForwardAction), systemImage: "arrowshape.turn.up.right")
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.forwardMessageAction)

        let isPinned = pinnedMessages.contains(where: { $0.messageId == message.messageId })
        Button {
            Task { @MainActor in
                if isPinned {
                    _ = await store.unpinMessage(chatId: chatId, messageId: message.messageId, router: router, locale: theme.locale)
                } else {
                    _ = await store.pinMessage(chatId: chatId, messageId: message.messageId, router: router, locale: theme.locale)
                }
            }
        } label: {
            Label(
                isPinned
                    ? theme.messagingString(.messageUnpinAction)
                    : theme.messagingString(.messagePinAction),
                systemImage: isPinned ? "pin.slash" : "pin"
            )
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.pinMessageAction)

        Button {
            isMultiSelectMode = true
            selectedMessageIds.insert(message.messageId)
        } label: {
            Label(theme.messagingString(.messageSelectAction), systemImage: "checkmark.circle")
        }

        Divider()

        Button(role: .destructive) {
            Task { @MainActor in
                _ = await store.deleteMessages(
                    chatId: chatId,
                    messageIds: [message.messageId],
                    forEveryone: true,
                    router: router,
                    isCompact: horizontalSizeClass == .compact,
                    locale: theme.locale
                )
            }
        } label: {
            Label(theme.messagingString(.messageDeleteForEveryone), systemImage: "trash")
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.deleteMessageAction)

        Button(role: .destructive) {
            Task { @MainActor in
                _ = await store.deleteMessages(
                    chatId: chatId,
                    messageIds: [message.messageId],
                    forEveryone: false,
                    router: router,
                    isCompact: horizontalSizeClass == .compact,
                    locale: theme.locale
                )
            }
        } label: {
            Label(theme.messagingString(.messageDeleteForMe), systemImage: "trash.slash")
        }
    }

    // MARK: - Multi-Select Toolbar

    private func multiSelectToolbar(chatId: Int64) -> some View {
        HStack(spacing: 12) {
            Button(theme.localized("hig.common.cancel")) {
                isMultiSelectMode = false
                selectedMessageIds.removeAll()
            }
            .fabushiFont(.subheadline)
            .fabushiMinimumTouchTarget()

            Spacer()

            Text(theme.messagingFormat(.multiSelectCountFormat, selectedMessageIds.count))
                .fabushiFont(.subheadline)
                .fontWeight(.semibold)
                .foregroundStyle(theme.textPrimary)

            Spacer()

            Button {
                forwardSourceMessageIds = Array(selectedMessageIds)
                isForwardPickerPresented = true
            } label: {
                Label(theme.messagingString(.multiSelectForwardAction), systemImage: "arrowshape.turn.up.right")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .disabled(selectedMessageIds.isEmpty)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.multiSelectForwardButton)

            Button(role: .destructive) {
                let ids = Array(selectedMessageIds)
                isMultiSelectMode = false
                selectedMessageIds.removeAll()
                Task { @MainActor in
                    _ = await store.deleteMessages(
                        chatId: chatId,
                        messageIds: ids,
                        forEveryone: true,
                        router: router,
                        isCompact: horizontalSizeClass == .compact,
                        locale: theme.locale
                    )
                }
            } label: {
                Label(theme.messagingString(.multiSelectDeleteAction), systemImage: "trash")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.destructiveColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .disabled(selectedMessageIds.isEmpty)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.multiSelectDeleteButton)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(theme.surfaceComposer)
        .overlay(alignment: .top) {
            Divider().overlay(theme.separatorHairline)
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.multiSelectToolbar)
    }

    // MARK: - Stage Sheets (Pins, Forward Picker, Auto-Delete TTL `D10-08`)

    private func pinnedMessagesSheet(chatId: Int64) -> some View {
        NavigationStack {
            List(pinnedMessages) { pin in
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(pin.previewText.isEmpty ? "#\(pin.messageId)" : pin.previewText)
                            .fabushiFont(.body)
                            .foregroundStyle(theme.textPrimary)
                        Text("#\(pin.messageId)")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                    }
                    Spacer()
                    Button(role: .destructive) {
                        Task { @MainActor in
                            _ = await store.unpinMessage(chatId: chatId, messageId: pin.messageId, router: router, locale: theme.locale)
                        }
                    } label: {
                        Text(theme.messagingString(.messageUnpinAction))
                            .fabushiFont(.caption)
                    }
                }
            }
            .navigationTitle(theme.messagingString(.pinnedMessagesSheetTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        isPinsSheetPresented = false
                    }
                }
            }
        }
    }

    private func forwardTargetPickerSheet(fromChatId: Int64) -> some View {
        NavigationStack {
            List(store.chats) { destination in
                Button {
                    let idsToForward = forwardSourceMessageIds
                    isForwardPickerPresented = false
                    isMultiSelectMode = false
                    selectedMessageIds.removeAll()
                    Task { @MainActor in
                        _ = await store.forwardMessages(
                            fromChatId: fromChatId,
                            messageIds: idsToForward,
                            toChatId: destination.chatId,
                            router: router,
                            isCompact: horizontalSizeClass == .compact,
                            locale: theme.locale
                        )
                    }
                } label: {
                    HStack(spacing: 12) {
                        FabushiAvatarBadge(
                            peerId: destination.chatId,
                            title: destination.title,
                            isOnline: false,
                            isBot: destination.isBot
                        )
                        Text(destination.title)
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.textPrimary)
                        Spacer()
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
            }
            .navigationTitle(theme.messagingString(.forwardPickerTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel")) {
                        isForwardPickerPresented = false
                    }
                }
            }
        }
    }

    private func ttlPickerSheet(chatId: Int64) -> some View {
        let currentTTL = activeChat?.ttlSeconds ?? 0
        let options: [(UInt32, FabushiMessagingStrings.Key)] = [
            (0, .ttlOff),
            (86_400, .ttlOneDay),
            (604_800, .ttlOneWeek),
            (2_592_000, .ttlOneMonth),
        ]

        return NavigationStack {
            List(options, id: \.0) { seconds, key in
                Button {
                    isTTLPickerPresented = false
                    Task { @MainActor in
                        _ = await store.setChatTTL(
                            chatId: chatId,
                            ttlSeconds: seconds,
                            router: router,
                            isCompact: horizontalSizeClass == .compact,
                            locale: theme.locale
                        )
                    }
                } label: {
                    HStack {
                        Text(theme.messagingString(key))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.textPrimary)
                        Spacer()
                        if currentTTL == seconds {
                            Image(systemName: "checkmark")
                                .fabushiFont(.body)
                                .foregroundStyle(theme.accentColor)
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
            }
            .navigationTitle(theme.messagingString(.ttlTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        isTTLPickerPresented = false
                    }
                }
            }
        }
    }

    private func toggleMultiSelect(_ messageId: Int64) {
        if selectedMessageIds.contains(messageId) {
            selectedMessageIds.remove(messageId)
        } else {
            selectedMessageIds.insert(messageId)
        }
    }

    private func openInspectorForCurrentChat(chatId: Int64) {
        if let chat = activeChat {
            if chat.isBot {
                router.openInspector(for: .bot(botId: chatId, chatId: chatId))
            } else if chat.isGroup || chat.isChannel {
                router.openInspector(for: .group(chatId: chatId))
            } else {
                router.openInspector(for: .user(peerId: chatId))
            }
        } else {
            router.openInspector(for: .chat(chatId: chatId))
        }
    }
}

private extension FabushiMessagingStrings.Key {
    static var reactMessageActionLabel: FabushiMessagingStrings.Key { .stickerTabEmoji }
}
