import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - `D13-04` 2-Wave Parallel Context Inspector Snapshot & Loader

public struct FabushiInspectorSnapshot: Sendable, Equatable {
    public var chat: FabushiMessagingChat?
    public var members: [FabushiChatMember]
    public var invites: [FabushiInviteLink]
    public var pins: [FabushiPinnedMessageRecord]
    public var draft: FabushiDraftRecord?
    public var statistics: FabushiChannelStatistics?
    public var sendAsPeers: [FabushiSendAsPeer]
    public var modelCatalog: FabushiAiModelCatalog
    public var botConfig: FabushiBotConfig?
    public var botComputerProfile: FabushiBotComputerProfile?
    public var computers: [FabushiBotComputer]
    public var pendingApprovals: [FabushiApprovalItem]
    public var botPublicProfile: FabushiBotPublicProfile?
    public var userProfile: FabushiUserProfile?
    public var wave1CompletedCount: Int
    public var wave2CompletedCount: Int

    public init(
        chat: FabushiMessagingChat? = nil,
        members: [FabushiChatMember] = [],
        invites: [FabushiInviteLink] = [],
        pins: [FabushiPinnedMessageRecord] = [],
        draft: FabushiDraftRecord? = nil,
        statistics: FabushiChannelStatistics? = nil,
        sendAsPeers: [FabushiSendAsPeer] = [],
        modelCatalog: FabushiAiModelCatalog = FabushiAiModelCatalog(),
        botConfig: FabushiBotConfig? = nil,
        botComputerProfile: FabushiBotComputerProfile? = nil,
        computers: [FabushiBotComputer] = [],
        pendingApprovals: [FabushiApprovalItem] = [],
        botPublicProfile: FabushiBotPublicProfile? = nil,
        userProfile: FabushiUserProfile? = nil,
        wave1CompletedCount: Int = 0,
        wave2CompletedCount: Int = 0
    ) {
        self.chat = chat
        self.members = members
        self.invites = invites
        self.pins = pins
        self.draft = draft
        self.statistics = statistics
        self.sendAsPeers = sendAsPeers
        self.modelCatalog = modelCatalog
        self.botConfig = botConfig
        self.botComputerProfile = botComputerProfile
        self.computers = computers
        self.pendingApprovals = pendingApprovals
        self.botPublicProfile = botPublicProfile
        self.userProfile = userProfile
        self.wave1CompletedCount = wave1CompletedCount
        self.wave2CompletedCount = wave2CompletedCount
    }
}

/// `D13-04` 2-wave parallel metadata fetcher eliminating sequential inspector waterfalls.
@MainActor
public enum FabushiInspectorLoader {
    public static func load(
        target: InspectorTarget,
        store: FabushiMessagingStore
    ) async -> FabushiInspectorSnapshot {
        let capturedGen = store.coordinator.accountScopeGeneration
        let resolvedChatId: Int64?
        let explicitBotId: UInt64?
        let explicitUserId: UInt64?

        switch target {
        case .chat(let cid), .group(let cid):
            resolvedChatId = cid
            explicitBotId = nil
            explicitUserId = nil
        case .bot(let botId, let cid):
            resolvedChatId = cid ?? botId
            explicitBotId = botId > 0 ? UInt64(botId) : nil
            explicitUserId = nil
        case .user(let peerId):
            resolvedChatId = peerId
            explicitBotId = nil
            explicitUserId = peerId > 0 ? UInt64(peerId) : nil
        }

        // Wave 1: 8 independent metadata calls in parallel via `async let`
        async let chatTask: FabushiMessagingChat? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.getChat(chatId: cid)
        }()
        async let membersTask: [FabushiChatMember]? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.listMembers(chatId: cid)
        }()
        async let invitesTask: [FabushiInviteLink]? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.listInvites(chatId: cid)
        }()
        async let pinsTask: [FabushiPinnedMessageRecord]? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.listPinnedMessages(chatId: cid)
        }()
        async let draftTask: FabushiDraftRecord? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.getDraft(chatId: cid)
        }()
        async let statsTask: FabushiChannelStatistics? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.getChannelStatistics(chatId: cid)
        }()
        async let sendAsTask: [FabushiSendAsPeer]? = {
            guard let cid = resolvedChatId else { return nil }
            return try? await store.client.listSendAsPeers(chatId: cid)
        }()
        async let catalogTask: FabushiAiModelCatalog = store.client.fetchEffectiveAiModelCatalog()

        let (fetchedChat, fetchedMembers, fetchedInvites, fetchedPins, fetchedDraft, fetchedStats, fetchedSendAs, fetchedCatalog) = await (
            chatTask,
            membersTask,
            invitesTask,
            pinsTask,
            draftTask,
            statsTask,
            sendAsTask,
            catalogTask
        )

        guard store.coordinator.accountScopeGeneration == capturedGen else {
            return FabushiInspectorSnapshot()
        }

        var wave1Count = 0
        if fetchedChat != nil { wave1Count += 1 }
        if fetchedMembers != nil { wave1Count += 1 }
        if fetchedInvites != nil { wave1Count += 1 }
        if fetchedPins != nil { wave1Count += 1 }
        if fetchedDraft != nil { wave1Count += 1 }
        if fetchedStats != nil { wave1Count += 1 }
        if fetchedSendAs != nil { wave1Count += 1 }
        if fetchedCatalog.hasConfiguredProvider || !fetchedCatalog.routingGroups.isEmpty { wave1Count += 1 }

        let fallbackChat = resolvedChatId.flatMap { cid in store.chats.first(where: { $0.chatId == cid }) }
        let effectiveChat = fetchedChat ?? fallbackChat

        // Wave 2: dependent bot/computer/approvals/user profile/read-sync calls in parallel
        let resolvedBotId: UInt64? = explicitBotId
            ?? ((effectiveChat?.isBot == true && (effectiveChat?.chatId ?? 0) > 0) ? UInt64(effectiveChat!.chatId) : nil)
        let botTargetHandle: String? = effectiveChat?.username ?? resolvedBotId.map(String.init)

        async let botConfigTask: FabushiBotConfig? = {
            guard let bid = resolvedBotId else { return nil }
            return try? await store.client.getBot(botId: bid)
        }()
        async let botComputerTask: FabushiBotComputerProfile? = {
            guard let bid = resolvedBotId else { return nil }
            return try? await store.client.getBotComputerProfile(botId: bid)
        }()
        async let computersTask: [FabushiBotComputer]? = {
            guard resolvedBotId != nil else { return nil }
            return try? await store.client.listComputers()
        }()
        async let approvalsTask: [FabushiApprovalItem]? = try? await store.client.listApprovals(pendingOnly: true)
        async let botPublicTask: FabushiBotPublicProfile? = {
            guard let target = botTargetHandle else { return nil }
            return try? await store.client.getBotPublicProfile(target: target)
        }()
        async let userProfileTask: FabushiUserProfile? = {
            guard let uid = explicitUserId else { return nil }
            return try? await store.client.getUserProfile(userId: uid)
        }()

        let (botCfg, botComp, allComputers, approvals, botPub, userProf) = await (
            botConfigTask,
            botComputerTask,
            computersTask,
            approvalsTask,
            botPublicTask,
            userProfileTask
        )

        guard store.coordinator.accountScopeGeneration == capturedGen else {
            return FabushiInspectorSnapshot()
        }

        var wave2Count = 0
        if botCfg != nil { wave2Count += 1 }
        if botComp != nil { wave2Count += 1 }
        if allComputers != nil { wave2Count += 1 }
        if approvals != nil { wave2Count += 1 }
        if botPub != nil { wave2Count += 1 }
        if userProf != nil { wave2Count += 1 }

        if let effectiveChat {
            store.upsertChat(effectiveChat)
        }

        return FabushiInspectorSnapshot(
            chat: effectiveChat,
            members: fetchedMembers ?? [],
            invites: fetchedInvites ?? [],
            pins: fetchedPins ?? [],
            draft: fetchedDraft,
            statistics: fetchedStats,
            sendAsPeers: fetchedSendAs ?? [],
            modelCatalog: fetchedCatalog,
            botConfig: botCfg,
            botComputerProfile: botComp,
            computers: allComputers ?? [],
            pendingApprovals: approvals ?? [],
            botPublicProfile: botPub,
            userProfile: userProf,
            wave1CompletedCount: wave1Count,
            wave2CompletedCount: wave2Count
        )
    }
}

// MARK: - Muse Context Inspector View (`#17`, `D10-08`, `D10-09`, `D13-04`, `#19`)

public enum FabushiInspectorSharedTab: String, CaseIterable, Sendable, Identifiable {
    case members
    case media
    case files
    case links
    case voice

    public var id: String { rawValue }
}

@MainActor
public struct FabushiContextInspectorView: View {
    public let target: InspectorTarget
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var snapshot = FabushiInspectorSnapshot()
    @State private var isLoading: Bool = false
    @State private var selectedTab: FabushiInspectorSharedTab = .media
    @State private var copiedUsernameFeedback: Bool = false
    @State private var isLeaveConfirmPresented: Bool = false
    @State private var isEditInfoPresented: Bool = false
    @State private var editTitleText: String = ""
    @State private var editAboutText: String = ""
    @State private var isAddMemberPresented: Bool = false
    @State private var addMemberUserIdText: String = ""
    @State private var isSafetySheetPresented: Bool = false
    @State private var isStatisticsSheetPresented: Bool = false
    @State private var exportedJSON: String? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var resolvedChatId: Int64 {
        switch target {
        case .chat(let cid), .group(let cid): return cid
        case .bot(let bid, let cid): return cid ?? bid
        case .user(let pid): return pid
        }
    }

    private var activeChat: FabushiMessagingChat? {
        store.chats.first(where: { $0.chatId == resolvedChatId }) ?? snapshot.chat
    }

    private var displayTitle: String {
        if let title = activeChat?.title, !title.isEmpty { return title }
        if let botName = snapshot.botConfig?.displayName, !botName.isEmpty { return botName }
        if let userName = snapshot.userProfile?.displayName, !userName.isEmpty { return userName }
        return theme.localized("hig.inspector.title")
    }

    private var displayUsername: String? {
        activeChat?.username ?? snapshot.botConfig?.username ?? snapshot.userProfile?.username
    }

    private var displayAbout: String? {
        activeChat?.about ?? snapshot.botConfig?.descriptionText ?? snapshot.userProfile?.bio
    }

    private var isBotTarget: Bool {
        if case .bot = target { return true }
        return activeChat?.isBot == true
    }

    private var isGroupTarget: Bool {
        if case .group = target { return true }
        return activeChat?.isGroup == true || activeChat?.isChannel == true
    }

    private var resolvedPeerUserId: UInt64? {
        switch target {
        case .user(let pid): return pid > 0 ? UInt64(pid) : nil
        case .bot(let bid, _): return bid > 0 ? UInt64(bid) : nil
        default:
            return resolvedChatId > 0 ? UInt64(resolvedChatId) : nil
        }
    }

    public init(
        target: InspectorTarget,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil
    ) {
        self.target = target
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
    }

    public var body: some View {
        NavigationStack {
            List {
                heroHeaderSection
                profileMetadataSection
                if isBotTarget {
                    botActionsSection
                }
                if isGroupTarget {
                    groupManagementSection
                } else {
                    userSecuritySection
                }
                settingsAndActionsSection
                sharedContentSection
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(theme.surfaceSecondary)
            .navigationTitle(theme.localized("hig.inspector.title"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button {
                        router.dismissInspector()
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiFont(.body)
                            .fabushiMinimumTouchTarget()
                    }
                    .accessibilityLabel(theme.localized("hig.common.close"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.closeButton)
                }
            }
            .task(id: target) {
                isLoading = true
                let loaded = await FabushiInspectorLoader.load(target: target, store: store)
                snapshot = loaded
                if loaded.chat?.isGroup == true || loaded.chat?.isChannel == true {
                    selectedTab = .members
                }
                isLoading = false
            }
            .sheet(isPresented: $isSafetySheetPresented) {
                FabushiE2EESafetyView(
                    chatId: resolvedChatId,
                    peerTitle: displayTitle,
                    remoteIdentityKeyB64: nil,
                    store: store,
                    router: router,
                    onDismiss: { isSafetySheetPresented = false }
                )
            }
            .sheet(isPresented: $isEditInfoPresented) {
                editInfoSheet
            }
            .sheet(isPresented: $isAddMemberPresented) {
                addMemberSheet
            }
            .sheet(isPresented: $isStatisticsSheetPresented) {
                FabushiInspectorStatisticsSheet(
                    title: displayTitle,
                    stats: snapshot.statistics,
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
            .alert(
                theme.messagingString(.inspectorLeaveConfirmTitle),
                isPresented: $isLeaveConfirmPresented
            ) {
                Button(theme.localized("hig.common.cancel"), role: .cancel) {
                    isLeaveConfirmPresented = false
                }
                .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.leaveConfirmCancel)

                Button(theme.messagingString(.leaveGroupAction), role: .destructive) {
                    Task { @MainActor in
                        _ = await store.leaveChat(
                            chatId: resolvedChatId,
                            router: router,
                            locale: theme.locale
                        )
                    }
                }
            } message: {
                Text(theme.messagingString(.inspectorLeaveConfirmMessage))
            }
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.drawer)
    }

    // MARK: - 80pt Centered Hero & 4-Icon Action Bar

    private var heroHeaderSection: some View {
        Section {
            VStack(spacing: 14) {
                FabushiAvatarBadge(
                    peerId: resolvedChatId,
                    title: displayTitle,
                    isOnline: false,
                    isBot: isBotTarget
                )
                .scaleEffect(80.0 / 44.0)
                .frame(width: 80, height: 80)

                VStack(spacing: 4) {
                    Text(displayTitle)
                        .fabushiFont(.title2)
                        .foregroundStyle(theme.textPrimary)
                        .multilineTextAlignment(.center)

                    if let username = displayUsername, !username.isEmpty {
                        Button {
                            #if canImport(UIKit)
                            UIPasteboard.general.string = "@\(username)"
                            #endif
                            copiedUsernameFeedback = true
                            router.toastController.showStagePill(
                                theme.messagingString(.inspectorUsernameCopiedPill),
                                chatId: resolvedChatId
                            )
                        } label: {
                            HStack(spacing: 4) {
                                Text("@\(username)")
                                    .fabushiFont(.subheadline)
                                    .foregroundStyle(theme.accentColor)
                                Image(systemName: copiedUsernameFeedback ? "checkmark" : "doc.on.doc")
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(copiedUsernameFeedback ? theme.successColor : theme.textMuted)
                            }
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(theme.messagingString(.inspectorCopyUsernameAction))
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.copyUsernameButton)

                        if copiedUsernameFeedback {
                            Text(theme.messagingString(.inspectorUsernameCopiedPill))
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.successColor)
                                .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.copiedFeedback)
                        }
                    }
                }

                // 4-Icon Apple HIG Action Bar
                HStack(spacing: 12) {
                    let isMuted = activeChat?.isMuted ?? false
                    heroActionButton(
                        title: isMuted
                            ? theme.messagingString(.inspectorActionUnmute)
                            : theme.messagingString(.inspectorActionMute),
                        systemImage: isMuted ? "bell.slash.fill" : "bell.fill"
                    ) {
                        Task { @MainActor in
                            await store.toggleMute(chatId: resolvedChatId)
                        }
                    }

                    heroActionButton(
                        title: theme.messagingString(.inspectorActionSearch),
                        systemImage: "magnifyingglass"
                    ) {
                        router.dismissInspector()
                        store.overlayController.openUniversalSearch(inChatId: resolvedChatId)
                    }

                    if let mainAppId = snapshot.botPublicProfile?.mainAppId, !mainAppId.isEmpty {
                        heroActionButton(
                            title: theme.messagingString(.prestartBotOpenMiniAppAction),
                            systemImage: "square.grid.2x2.fill"
                        ) {
                            router.dismissInspector()
                            router.navigate(to: .miniApp(id: mainAppId), coordinator: coordinator)
                        }
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.openAppButton)
                    } else {
                        heroActionButton(
                            title: theme.messagingString(.inspectorActionCall),
                            systemImage: "phone.fill"
                        ) {
                            router.dismissInspector()
                            router.navigate(to: .calls, coordinator: coordinator)
                        }
                    }

                    heroActionButton(
                        title: theme.messagingString(.inspectorActionMore),
                        systemImage: "pencil"
                    ) {
                        editTitleText = displayTitle
                        editAboutText = displayAbout ?? ""
                        isEditInfoPresented = true
                    }
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .listRowBackground(Color.clear)
        }
    }

    private func heroActionButton(
        title: String,
        systemImage: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            VStack(spacing: 6) {
                Image(systemName: systemImage)
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.accentColor)
                Text(title)
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 10)
            .background(
                theme.surfaceElevated,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
            )
            .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
    }

    // MARK: - Profile Bio & Role Section

    @ViewBuilder
    private var profileMetadataSection: some View {
        let sectionID: String = isBotTarget
            ? FabushiAccessibilityIDs.Inspector.profileBotSection
            : (isGroupTarget ? FabushiAccessibilityIDs.Inspector.profileGroupSection : FabushiAccessibilityIDs.Inspector.profileUserSection)

        Section(header: Text(theme.messagingString(.inspectorProfileBioLabel))) {
            if let about = displayAbout, !about.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(about)
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textPrimary)
            } else {
                Text(isLoading ? theme.messagingString(.updatingConversations) : displayTitle)
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
            }

            if isGroupTarget {
                HStack {
                    Image(systemName: "person.3.sequence.fill")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.accentColor)
                    Text(theme.messagingString(.groupBadge))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    Text("\(max(snapshot.members.count, activeChat?.memberCount ?? 0))")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
                .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.multiAgentWorkspaceBadge)
            }
        }
        .accessibilityIdentifier(sectionID)
    }

    // MARK: - Bot Config & Computers Subpages (`D10-09`)

    private var botActionsSection: some View {
        Section(header: Text(theme.messagingString(.inspectorBotConfigTitle))) {
            let botId = resolvedPeerUserId ?? UInt64(max(0, resolvedChatId))
            NavigationLink {
                FabushiInspectorAiModelView(
                    botId: botId,
                    initialBotConfig: snapshot.botConfig,
                    initialComputerProfile: snapshot.botComputerProfile,
                    initialCatalog: snapshot.modelCatalog,
                    computers: snapshot.computers,
                    coordinator: coordinator,
                    router: router,
                    store: store,
                    onSaved: { updatedBot, updatedProfile in
                        if let updatedBot { snapshot.botConfig = updatedBot }
                        if let updatedProfile { snapshot.botComputerProfile = updatedProfile }
                    }
                )
            } label: {
                HStack {
                    Label(theme.messagingString(.inspectorBotConfigTitle), systemImage: "cpu")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    let normalized = FabushiAiModelCatalog.normalizePreferredModel(
                        (snapshot.botComputerProfile?.updatedAtMs ?? 0) > 0
                            ? snapshot.botComputerProfile?.preferredModel
                            : snapshot.botConfig?.preferredModel
                    )
                    Text(normalized.isEmpty ? theme.messagingString(.inspectorFollowSettingsOption) : normalized)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                        .lineLimit(1)
                }
                .fabushiMinimumTouchTarget()
            }

            Button {
                router.dismissInspector()
                router.navigate(
                    to: .remoteComputer(nodeId: snapshot.botComputerProfile?.boundNodeId),
                    coordinator: coordinator
                )
            } label: {
                Label(theme.messagingString(.inspectorBotComputersTitle), systemImage: "desktopcomputer")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.codingHostProbeButton)
        }
    }

    // MARK: - Group Invites & Leave Group

    private var groupManagementSection: some View {
        Section(header: Text(theme.messagingString(.inspectorInviteLinksTitle))) {
            Button {
                isAddMemberPresented = true
            } label: {
                Label(theme.messagingString(.inspectorAddMemberAction), systemImage: "person.badge.plus")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)

            Button {
                Task { @MainActor in
                    if let created = try? await store.client.createInvite(chatId: resolvedChatId, title: displayTitle) {
                        snapshot.invites.insert(created, at: 0)
                    }
                }
            } label: {
                Label(theme.messagingString(.inspectorCreateInviteAction), systemImage: "link.badge.plus")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)

            ForEach(snapshot.invites) { invite in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(invite.link.isEmpty ? invite.code : invite.link)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textPrimary)
                            .lineLimit(1)
                        Text("\(invite.usageCount) uses")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                    }
                    Spacer()
                    Button(role: .destructive) {
                        Task { @MainActor in
                            try? await store.client.revokeInvite(chatId: resolvedChatId, inviteId: invite.inviteId)
                            snapshot.invites.removeAll { $0.inviteId == invite.inviteId }
                        }
                    } label: {
                        Text(theme.messagingString(.inspectorRevokeInviteAction))
                            .fabushiFont(.caption)
                    }
                }
            }

            Button(role: .destructive) {
                isLeaveConfirmPresented = true
            } label: {
                Label(theme.messagingString(.leaveGroupAction), systemImage: "rectangle.portrait.and.arrow.right")
                    .fabushiFont(.body)
                    .fabushiMinimumTouchTarget()
            }
            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.leaveGroupButton)
        }
    }

    // MARK: - User Security, E2EE & Blocklist (`#19`, `D10-08`)

    private var userSecuritySection: some View {
        Section(header: Text(theme.messagingString(.e2eeTitle))) {
            Button {
                isSafetySheetPresented = true
            } label: {
                HStack {
                    Label(theme.messagingString(.e2eeSafetyNumbersTitle), systemImage: "lock.shield")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    let verified = store.verifiedE2EEChatIds.contains(resolvedChatId)
                    Text(verified ? theme.messagingString(.e2eeVerifiedStatus) : theme.messagingString(.e2eeUnverifiedStatus))
                        .fabushiFont(.caption)
                        .foregroundStyle(verified ? theme.successColor : theme.textSecondary)
                }
                .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.startSecretChatButton)

            if let uid = resolvedPeerUserId {
                let blocked = store.isUserBlocked(uid)
                Button(role: blocked ? nil : .destructive) {
                    Task { @MainActor in
                        _ = await store.setUserBlocked(
                            userId: uid,
                            blocked: !blocked,
                            router: router,
                            locale: theme.locale
                        )
                    }
                } label: {
                    Label(
                        blocked
                            ? theme.messagingString(.unblockUserAction)
                            : theme.messagingString(.blockUserAction),
                        systemImage: blocked ? "hand.raised.slash" : "hand.raised.fill"
                    )
                    .fabushiFont(.body)
                    .fabushiMinimumTouchTarget()
                }
                .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.blockContactButton)
            }
        }
    }

    // MARK: - Auto-Delete TTL (`D10-08`), Statistics & Export

    private var settingsAndActionsSection: some View {
        Section(header: Text(theme.messagingString(.ttlTitle))) {
            let currentTTL = activeChat?.ttlSeconds ?? 0
            let ttlOptions: [(UInt32, FabushiMessagingStrings.Key)] = [
                (0, .ttlOff),
                (86_400, .ttlOneDay),
                (604_800, .ttlOneWeek),
                (2_592_000, .ttlOneMonth),
            ]
            Picker(theme.messagingString(.ttlTitle), selection: Binding(
                get: { currentTTL },
                set: { newTTL in
                    Task { @MainActor in
                        _ = await store.setChatTTL(
                            chatId: resolvedChatId,
                            ttlSeconds: newTTL,
                            router: router,
                            isCompact: horizontalSizeClass == .compact,
                            locale: theme.locale
                        )
                    }
                }
            )) {
                ForEach(ttlOptions, id: \.0) { seconds, key in
                    Text(theme.messagingString(key)).tag(seconds)
                }
            }

            Button {
                isStatisticsSheetPresented = true
            } label: {
                Label(theme.messagingString(.inspectorStatisticsAction), systemImage: "chart.bar.xaxis")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textPrimary)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)

            Button {
                Task { @MainActor in
                    do {
                        exportedJSON = try await store.exportChatHistoryJSON(chatId: resolvedChatId)
                        _ = router.showChatToastIfVisible(
                            theme.messagingString(.exportHistoryDoneToast),
                            chatId: resolvedChatId,
                            coordinator: coordinator,
                            isCompact: horizontalSizeClass == .compact,
                            isError: false
                        )
                    } catch {
                        _ = router.showChatToastIfVisible(
                            theme.messagingString(.exportHistoryFailedToast),
                            chatId: resolvedChatId,
                            coordinator: coordinator,
                            isCompact: horizontalSizeClass == .compact,
                            isError: true
                        )
                    }
                }
            } label: {
                Label(theme.messagingString(.inspectorExportChatAction), systemImage: "square.and.arrow.up")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textPrimary)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: - 5-Tab Shared Media & Members

    private var sharedContentSection: some View {
        Section {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    tabPill(.members, key: .inspectorMembersTab, id: FabushiAccessibilityIDs.Inspector.tabMembers)
                    tabPill(.media, key: .inspectorMediaTab, id: FabushiAccessibilityIDs.Inspector.tabMedia)
                    tabPill(.files, key: .inspectorFilesTab, id: FabushiAccessibilityIDs.Inspector.tabFiles)
                    tabPill(.links, key: .inspectorLinksTab, id: FabushiAccessibilityIDs.Inspector.tabLinks)
                    tabPill(.voice, key: .inspectorVoiceTab, id: FabushiAccessibilityIDs.Inspector.tabVoice)
                }
                .padding(.vertical, 4)
            }

            let chatMessages = store.messagesByChat[resolvedChatId] ?? []
            switch selectedTab {
            case .members:
                if snapshot.members.isEmpty {
                    Text(theme.messagingString(.inspectorNoMembers))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(snapshot.members) { member in
                        HStack(spacing: 10) {
                            FabushiAvatarBadge(
                                peerId: Int64(bitPattern: member.userId),
                                title: member.displayName,
                                isOnline: false,
                                isBot: false
                            )
                            VStack(alignment: .leading, spacing: 2) {
                                Text(member.displayName)
                                    .fabushiFont(.subheadline)
                                    .foregroundStyle(theme.textPrimary)
                                Text(member.adminTitle ?? member.role.capitalized)
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                            Spacer()
                        }
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) {
                                Task { @MainActor in
                                    _ = try? await store.client.removeMember(chatId: resolvedChatId, userId: member.userId)
                                    snapshot.members.removeAll { $0.userId == member.userId }
                                }
                            } label: {
                                Label(theme.messagingString(.deleteChatAction), systemImage: "person.badge.minus")
                            }
                        }
                    }
                }
            case .media:
                let items = chatMessages.filter { msg in
                    switch msg.content {
                    case .photo, .video, .media, .videoNote: return true
                    default: return false
                    }
                }
                sharedMessagesList(items, icon: "photo")
            case .files:
                let items = chatMessages.filter { msg in
                    if case .document = msg.content { return true }
                    return false
                }
                sharedMessagesList(items, icon: "doc.fill")
            case .links:
                let items = chatMessages.filter { msg in
                    msg.text.contains("http://") || msg.text.contains("https://")
                        || msg.entities.contains(where: {
                            if case .url = $0.entityType { return true }
                            if case .textUrl = $0.entityType { return true }
                            return false
                        })
                }
                sharedMessagesList(items, icon: "link")
            case .voice:
                let items = chatMessages.filter { msg in
                    switch msg.content {
                    case .voice, .audio: return true
                    default: return false
                    }
                }
                sharedMessagesList(items, icon: "waveform")
            }
        }
    }

    private func tabPill(
        _ tab: FabushiInspectorSharedTab,
        key: FabushiMessagingStrings.Key,
        id: String
    ) -> some View {
        let isSelected = selectedTab == tab
        return Button {
            selectedTab = tab
        } label: {
            Text(theme.messagingString(key))
                .fabushiFont(.caption)
                .fontWeight(isSelected ? .semibold : .regular)
                .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(isSelected ? theme.accentColor : theme.surfaceInput, in: Capsule())
                .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier(id)
    }

    @ViewBuilder
    private func sharedMessagesList(_ items: [FabushiMessageRecord], icon: String) -> some View {
        if items.isEmpty {
            Text(theme.messagingString(.inspectorNoSharedMedia))
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.textSecondary)
        } else {
            ForEach(items) { item in
                Button {
                    router.dismissInspector()
                    router.navigate(to: .chats(chatId: resolvedChatId, messageId: item.messageId), coordinator: coordinator)
                } label: {
                    HStack(spacing: 10) {
                        Image(systemName: icon)
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.accentColor)
                        Text(item.text.isEmpty ? (item.content?.previewSummary ?? "") : item.text)
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textPrimary)
                            .lineLimit(2)
                        Spacer()
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
            }
        }
    }

    // MARK: - Edit Info & Add Member Sheets

    private var editInfoSheet: some View {
        NavigationStack {
            Form {
                TextField(theme.messagingString(.inspectorChatTitlePlaceholder), text: $editTitleText)
                    .fabushiFont(.body)
                TextField(theme.messagingString(.inspectorChatAboutPlaceholder), text: $editAboutText, axis: .vertical)
                    .fabushiFont(.body)
                    .lineLimit(3...6)
            }
            .navigationTitle(theme.messagingString(.inspectorEditInfoTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel")) {
                        isEditInfoPresented = false
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.inspectorSaveChangesAction)) {
                        Task { @MainActor in
                            _ = try? await store.client.updateChatInfo(
                                chatId: resolvedChatId,
                                title: editTitleText,
                                about: editAboutText
                            )
                            if var updated = activeChat {
                                updated.title = editTitleText
                                updated.about = editAboutText
                                store.upsertChat(updated)
                                snapshot.chat = updated
                            }
                            isEditInfoPresented = false
                        }
                    }
                }
            }
        }
    }

    private var addMemberSheet: some View {
        NavigationStack {
            Form {
                TextField(theme.messagingString(.inspectorAddMemberPlaceholder), text: $addMemberUserIdText)
                    .fabushiFont(.body)
                    .keyboardType(.numberPad)
            }
            .navigationTitle(theme.messagingString(.inspectorAddMemberAction))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel")) {
                        isAddMemberPresented = false
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.localized("hig.common.apply")) {
                        guard let uid = UInt64(addMemberUserIdText.trimmingCharacters(in: .whitespacesAndNewlines)) else {
                            return
                        }
                        Task { @MainActor in
                            _ = try? await store.client.addMembers(chatId: resolvedChatId, userIds: [uid])
                            if let refreshed = try? await store.client.listMembers(chatId: resolvedChatId) {
                                snapshot.members = refreshed
                            }
                            addMemberUserIdText = ""
                            isAddMemberPresented = false
                        }
                    }
                    .disabled(UInt64(addMemberUserIdText.trimmingCharacters(in: .whitespacesAndNewlines)) == nil)
                }
            }
        }
    }
}

// MARK: - Chat Statistics Sheet (Reusable by Stage & Inspector)

@MainActor
public struct FabushiInspectorStatisticsSheet: View {
    public let title: String
    public let stats: FabushiChannelStatistics?
    public let onClose: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(title: String, stats: FabushiChannelStatistics?, onClose: @escaping () -> Void) {
        self.title = title
        self.stats = stats
        self.onClose = onClose
    }

    public var body: some View {
        NavigationStack {
            List {
                if let stats {
                    Section(header: Text(title)) {
                        statRow(theme.messagingString(.statsMembersLabel), value: "\(stats.memberCount)")
                        statRow(theme.messagingString(.statsTotalMessagesLabel), value: "\(stats.totalMessages)")
                        statRow(theme.messagingString(.statsLast7DaysLabel), value: "\(stats.messagesLast7d)")
                        statRow(theme.messagingString(.statsReactionsLabel), value: "\(stats.totalReactions)")
                        statRow(theme.messagingString(.statsActivePostersLabel), value: "\(stats.activePosters7d)")
                        statRow(theme.messagingString(.statsGrowthRateLabel), value: String(format: "%.1f%%", stats.growthRatePct))
                    }

                    if !stats.dailySeries.isEmpty {
                        Section(header: Text(theme.messagingString(.statsLast7DaysLabel))) {
                            ForEach(stats.dailySeries, id: \.day) { point in
                                HStack {
                                    Text(point.day)
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.textSecondary)
                                    Spacer()
                                    Text("\(point.messages)")
                                        .fabushiFont(.subheadline)
                                        .fontWeight(.semibold)
                                        .foregroundStyle(theme.textPrimary)
                                }
                            }
                        }
                    }
                } else {
                    Text(theme.messagingString(.inspectorNoSharedMedia))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)
                }
            }
            .navigationTitle(theme.messagingString(.chatStatisticsTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close"), action: onClose)
                }
            }
        }
    }

    private func statRow(_ label: String, value: String) -> some View {
        HStack {
            Text(label)
                .fabushiFont(.body)
                .foregroundStyle(theme.textPrimary)
            Spacer()
            Text(value)
                .fabushiFont(.headline)
                .foregroundStyle(theme.accentColor)
        }
    }
}
