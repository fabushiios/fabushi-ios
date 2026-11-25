import SwiftUI

// MARK: - Apple HIG Contacts, Blocklist, Send-As & Prestart Bot View (`#11`, `D10-08`)

/// Unified Apple HIG Contacts management view with `@username` peer resolution, CRUD,
/// authoritative blocklist (`D10-08`), Send-As identity selector, and Prestart Bot hero sheet.
///
/// Layout invariant: NEVER nests a second `NavigationStack` at the root view.
@MainActor
public struct FabushiContactsView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?

    @State private var searchText: String = ""
    @State private var isAddContactPresented: Bool = false
    @State private var isBlocklistPresented: Bool = false
    @State private var isSendAsSheetPresented: Bool = false
    @State private var prestartBotProfile: FabushiBotPublicProfile? = nil
    @State private var isResolvingUsername: Bool = false
    @State private var lookupNotFound: Bool = false

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

    private var filteredContacts: [FabushiContact] {
        let trimmed = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let cleaned = trimmed.hasPrefix("@") ? String(trimmed.dropFirst()) : trimmed
        let sorted = store.contacts.sorted {
            $0.displayName.localizedCaseInsensitiveCompare($1.displayName) == .orderedAscending
        }
        guard !cleaned.isEmpty else { return sorted }
        return sorted.filter { contact in
            contact.displayName.lowercased().contains(cleaned)
                || (contact.username?.lowercased().contains(cleaned) ?? false)
                || (contact.phone?.lowercased().contains(cleaned) ?? false)
        }
    }

    public var body: some View {
        VStack(spacing: 0) {
            searchAndActionHeader
            Divider().overlay(theme.separatorHairline)
            contactsContent
        }
        .background(theme.surfacePrimary)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    isBlocklistPresented = true
                } label: {
                    Image(systemName: "hand.raised")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.messagingString(.blockedUsersTitle))

                Button {
                    isAddContactPresented = true
                } label: {
                    Image(systemName: "person.badge.plus")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .accessibilityLabel(theme.messagingString(.addContactTitle))
                .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.addContactButton)
            }
        }
        .task(id: coordinator.accountScopeGeneration) {
            await store.refreshContactsAndBlocklist()
        }
        .sheet(isPresented: $isAddContactPresented) {
            FabushiAddContactSheetView(
                store: store,
                router: router,
                onDismiss: { isAddContactPresented = false }
            )
        }
        .sheet(isPresented: $isBlocklistPresented) {
            FabushiBlockedUsersSheetView(
                store: store,
                router: router,
                onDismiss: { isBlocklistPresented = false }
            )
        }
        .sheet(isPresented: $isSendAsSheetPresented) {
            if let activeId = store.activeChatId {
                FabushiSendAsIdentitySheetView(
                    chatId: activeId,
                    store: store,
                    onDismiss: { isSendAsSheetPresented = false }
                )
            }
        }
        .sheet(item: Binding(
            get: { prestartBotProfile.map { IdentifiableBotProfile(profile: $0) } },
            set: { if $0 == nil { prestartBotProfile = nil } }
        )) { wrapped in
            NavigationStack {
                FabushiPrestartBotHeroView(
                    profile: wrapped.profile,
                    store: store,
                    coordinator: coordinator,
                    router: router,
                    onStarted: { _ in
                        prestartBotProfile = nil
                    }
                )
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(theme.localized("hig.common.close")) {
                            prestartBotProfile = nil
                        }
                    }
                }
            }
        }
    }

    private struct IdentifiableBotProfile: Identifiable {
        let profile: FabushiBotPublicProfile
        var id: UInt64 { profile.bot.botId }
    }

    // MARK: - Search & Quick Utility Bar

    private var searchAndActionHeader: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textMuted)
                        .accessibilityHidden(true)

                    TextField(
                        theme.messagingString(.contactsSearchPlaceholder),
                        text: $searchText
                    )
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textPrimary)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .onSubmit {
                        Task { @MainActor in
                            await triggerPeerLookup()
                        }
                    }

                    if !searchText.isEmpty {
                        Button {
                            searchText = ""
                            lookupNotFound = false
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

                if !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Button {
                        Task { @MainActor in
                            await triggerPeerLookup()
                        }
                    } label: {
                        if isResolvingUsername {
                            ProgressView().controlSize(.small)
                                .fabushiMinimumTouchTarget()
                        } else {
                            Image(systemName: "at.circle.fill")
                                .fabushiFont(.title3)
                                .foregroundStyle(theme.accentColor)
                                .fabushiMinimumTouchTarget()
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.messagingString(.resolveUsernameTitle))
                }
            }

            HStack(spacing: 8) {
                Button {
                    isAddContactPresented = true
                } label: {
                    Label(theme.messagingString(.addContactTitle), systemImage: "person.badge.plus")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.accentColor)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(theme.surfaceInput, in: Capsule())
                }
                .buttonStyle(.plain)
                .fabushiMinimumTouchTarget()

                Button {
                    isBlocklistPresented = true
                } label: {
                    Label(
                        "\(theme.messagingString(.blockedUsersTitle)) (\(store.blockedUsers.count))",
                        systemImage: "hand.raised"
                    )
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.surfaceInput, in: Capsule())
                }
                .buttonStyle(.plain)
                .fabushiMinimumTouchTarget()

                if let activeId = store.activeChatId,
                   !(store.sendAsPeersByChat[activeId]?.isEmpty ?? true) {
                    Button {
                        isSendAsSheetPresented = true
                    } label: {
                        Label(theme.messagingString(.sendAsTitle), systemImage: "person.crop.circle.badge.checkmark")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(theme.surfaceInput, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .fabushiMinimumTouchTarget()
                }

                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    // MARK: - Main Contacts List

    @ViewBuilder
    private var contactsContent: some View {
        if store.isLoadingContacts && store.contacts.isEmpty && store.resolvedContactPeer == nil {
            FabushiDialogListSkeletonView()
        } else if filteredContacts.isEmpty && store.resolvedContactPeer == nil {
            FabushiEmptyStateView(
                title: lookupNotFound
                    ? theme.messagingString(.resolveUsernameNotFound)
                    : theme.localized("hig.empty.contactsTitle"),
                systemImage: "person.2",
                descriptionText: theme.localized("hig.empty.contactsDescription"),
                accessibilityID: FabushiAccessibilityIDs.ChatList.contactsList
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            List {
                if let resolved = store.resolvedContactPeer {
                    Section {
                        resolvedPeerRow(resolved)
                    } header: {
                        FabushiSectionHeader(theme.messagingString(.resolveUsernameTitle))
                    }
                }

                if !filteredContacts.isEmpty {
                    Section {
                        ForEach(filteredContacts) { contact in
                            contactRow(contact)
                        }
                    } header: {
                        FabushiSectionHeader(theme.messagingString(.contactsTitle))
                    }
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .refreshable {
                await store.refreshContactsAndBlocklist()
            }
            .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.contactsList)
        }
    }

    private func resolvedPeerRow(_ profile: FabushiUserProfile) -> some View {
        HStack(spacing: 12) {
            FabushiAvatarBadge(
                peerId: Int64(bitPattern: profile.userId),
                title: profile.displayName,
                isOnline: profile.status.lowercased() == "online",
                isBot: profile.isBot
            )

            VStack(alignment: .leading, spacing: 2) {
                Text(profile.displayName)
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                if let username = profile.username, !username.isEmpty {
                    Text("@\(username)")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
            }

            Spacer(minLength: 8)

            Button {
                openPeer(userId: profile.userId, username: profile.username, displayName: profile.displayName, isBot: profile.isBot, existingChatId: nil)
            } label: {
                Text(theme.messagingString(.startChatAction))
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.accentForegroundColor)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(theme.accentColor, in: Capsule())
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
        }
        .listRowBackground(theme.surfacePrimary)
    }

    private func contactRow(_ contact: FabushiContact) -> some View {
        let isBlocked = store.isUserBlocked(contact.userId)
        return Button {
            openPeer(
                userId: contact.userId,
                username: contact.username,
                displayName: contact.displayName,
                isBot: contact.isBot,
                existingChatId: contact.chatId
            )
        } label: {
            HStack(spacing: 12) {
                FabushiAvatarBadge(
                    peerId: Int64(bitPattern: contact.userId),
                    title: contact.displayName,
                    isOnline: false,
                    isBot: contact.isBot
                )

                VStack(alignment: .leading, spacing: 2) {
                    Text(contact.displayName)
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)

                    if let username = contact.username, !username.isEmpty {
                        Text("@\(username)")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .lineLimit(1)
                    } else if let phone = contact.phone, !phone.isEmpty {
                        Text(phone)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .lineLimit(1)
                    }
                }

                Spacer(minLength: 8)

                if isBlocked {
                    Image(systemName: "hand.raised.fill")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.destructiveColor)
                        .accessibilityLabel(theme.messagingString(.blockedUsersTitle))
                }
            }
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fabushiMinimumTouchTarget()
        .listRowBackground(theme.surfacePrimary)
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            Button(role: .destructive) {
                Task { @MainActor in
                    _ = await store.deleteContact(
                        userId: contact.userId,
                        router: router,
                        locale: theme.locale
                    )
                }
            } label: {
                Label(theme.messagingString(.contactDeleteAction), systemImage: "trash")
            }

            Button {
                Task { @MainActor in
                    _ = await store.setUserBlocked(
                        userId: contact.userId,
                        blocked: !isBlocked,
                        router: router,
                        locale: theme.locale
                    )
                }
            } label: {
                Label(
                    isBlocked
                        ? theme.messagingString(.unblockUserAction)
                        : theme.messagingString(.blockUserAction),
                    systemImage: isBlocked ? "hand.raised.slash" : "hand.raised"
                )
            }
            .tint(theme.warningColor)
        }
    }

    private func triggerPeerLookup() async {
        let trimmed = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        isResolvingUsername = true
        lookupNotFound = false
        let found = await store.resolveUsernamePeer(trimmed)
        isResolvingUsername = false
        lookupNotFound = (found == nil)
    }

    private func openPeer(
        userId: UInt64,
        username: String?,
        displayName: String,
        isBot: Bool,
        existingChatId: Int64?
    ) {
        if isBot {
            let target = (username?.isEmpty == false) ? username! : String(userId)
            Task { @MainActor in
                if let profile = await store.loadBotPublicProfile(target: target),
                   !profile.hasStartedChat {
                    prestartBotProfile = profile
                    return
                }
                let chatId = existingChatId ?? Int64(bitPattern: userId)
                router.navigate(to: .chats(chatId: chatId, messageId: nil), coordinator: coordinator)
                await store.selectChat(chatId)
            }
            return
        }

        if let existingChatId, existingChatId != 0 {
            router.navigate(to: .chats(chatId: existingChatId, messageId: nil), coordinator: coordinator)
            Task { @MainActor in
                await store.selectChat(existingChatId)
            }
            return
        }

        Task { @MainActor in
            let request = FabushiCreateChatRequest(
                title: displayName,
                chatType: 0,
                peerUserId: userId,
                username: username,
                memberIds: [userId]
            )
            if let created = try? await store.client.createChat(request) {
                store.upsertChat(created)
                _ = await store.subscribeWsIfNeeded(chatId: created.chatId, force: true)
                router.navigate(to: .chats(chatId: created.chatId, messageId: nil), coordinator: coordinator)
                await store.selectChat(created.chatId)
            }
        }
    }
}

// MARK: - Add Contact Sheet

@MainActor
private struct FabushiAddContactSheetView: View {
    let store: FabushiMessagingStore
    let router: ShellNavigationRouter
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var firstName: String = ""
    @State private var lastName: String = ""
    @State private var username: String = ""
    @State private var phone: String = ""
    @State private var isSaving: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(theme.messagingString(.contactFirstNamePlaceholder), text: $firstName)
                        .fabushiFont(.body)
                    TextField(theme.messagingString(.contactLastNamePlaceholder), text: $lastName)
                        .fabushiFont(.body)
                }

                Section {
                    TextField(theme.messagingString(.contactUsernamePlaceholder), text: $username)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled(true)
                        .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.contactUsernameField)

                    TextField(theme.messagingString(.contactPhonePlaceholder), text: $phone)
                        .fabushiFont(.body)
                        .keyboardType(.phonePad)
                }
            }
            .navigationTitle(theme.messagingString(.addContactTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onDismiss)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.contactSaveAction)) {
                        guard !isSaving else { return }
                        isSaving = true
                        Task { @MainActor in
                            let saved = await store.addContact(
                                firstName: firstName,
                                lastName: lastName,
                                username: username,
                                phone: phone,
                                router: router,
                                locale: theme.locale
                            )
                            isSaving = false
                            if saved != nil {
                                onDismiss()
                            }
                        }
                    }
                    .disabled(firstName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isSaving)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.contactSaveButton)
                }
            }
        }
    }
}

// MARK: - Blocklist Sheet (`D10-08`)

@MainActor
private struct FabushiBlockedUsersSheetView: View {
    let store: FabushiMessagingStore
    let router: ShellNavigationRouter
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        NavigationStack {
            Group {
                if store.blockedUsers.isEmpty {
                    FabushiEmptyStateView(
                        title: theme.messagingString(.blockedUsersTitle),
                        systemImage: "hand.raised.slash",
                        descriptionText: theme.messagingString(.blockedUsersEmpty),
                        accessibilityID: "fabushi.contacts.blockedEmpty"
                    )
                } else {
                    List(store.blockedUsers) { user in
                        HStack(spacing: 12) {
                            FabushiAvatarBadge(
                                peerId: Int64(bitPattern: user.userId),
                                title: user.displayName,
                                isOnline: false,
                                isBot: false
                            )
                            VStack(alignment: .leading, spacing: 2) {
                                Text(user.displayName)
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.textPrimary)
                                if let username = user.username, !username.isEmpty {
                                    Text("@\(username)")
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.textSecondary)
                                }
                            }
                            Spacer()
                            Button {
                                Task { @MainActor in
                                    _ = await store.setUserBlocked(
                                        userId: user.userId,
                                        blocked: false,
                                        router: router,
                                        locale: theme.locale
                                    )
                                }
                            } label: {
                                Text(theme.messagingString(.unblockUserAction))
                                    .fabushiFont(.caption)
                                    .fontWeight(.semibold)
                                    .foregroundStyle(theme.accentColor)
                                    .padding(.horizontal, 12)
                                    .padding(.vertical, 6)
                                    .background(theme.surfaceInput, in: Capsule())
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .listStyle(.plain)
                }
            }
            .navigationTitle(theme.messagingString(.blockedUsersTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close"), action: onDismiss)
                }
            }
        }
    }
}

// MARK: - Send-As Identity Selector Sheet (`#11`)

@MainActor
public struct FabushiSendAsIdentitySheetView: View {
    public let chatId: Int64
    public let store: FabushiMessagingStore
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(chatId: Int64, store: FabushiMessagingStore, onDismiss: @escaping () -> Void) {
        self.chatId = chatId
        self.store = store
        self.onDismiss = onDismiss
    }

    public var body: some View {
        let peers = store.sendAsPeersByChat[chatId] ?? []
        let currentPeerId = store.selectedSendAsPeerByChat[chatId]

        NavigationStack {
            List(peers) { peer in
                let isSelected = currentPeerId == peer.peerId || (currentPeerId == nil && peer.isDefault)
                Button {
                    store.selectSendAsPeer(chatId: chatId, peerId: peer.peerId)
                    onDismiss()
                } label: {
                    HStack(spacing: 12) {
                        FabushiAvatarBadge(
                            peerId: Int64(bitPattern: peer.peerId),
                            title: peer.title,
                            isOnline: false,
                            isBot: peer.kind.lowercased() == "bot"
                        )
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(peer.title)
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.textPrimary)
                                if peer.isDefault {
                                    Text(theme.messagingString(.sendAsDefaultBadge))
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.accentColor)
                                        .padding(.horizontal, 6)
                                        .padding(.vertical, 2)
                                        .background(theme.accentColor.opacity(0.14), in: Capsule())
                                }
                            }
                            if let username = peer.username, !username.isEmpty {
                                Text("@\(username)")
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                        }
                        Spacer()
                        if isSelected {
                            Image(systemName: "checkmark")
                                .fabushiFont(.body)
                                .foregroundStyle(theme.accentColor)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .fabushiMinimumTouchTarget()
            }
            .listStyle(.plain)
            .navigationTitle(theme.messagingString(.sendAsTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close"), action: onDismiss)
                }
            }
        }
    }
}

// MARK: - Virtual Prestart Bot Hero Card (`#11`, Reusable by Contacts & Conversation Stage)

@MainActor
public struct FabushiPrestartBotHeroView: View {
    public let profile: FabushiBotPublicProfile
    public let store: FabushiMessagingStore
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let onStarted: ((Int64) -> Void)?

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var isStarting: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        profile: FabushiBotPublicProfile,
        store: FabushiMessagingStore,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        onStarted: ((Int64) -> Void)? = nil
    ) {
        self.profile = profile
        self.store = store
        self.coordinator = coordinator
        self.router = router
        self.onStarted = onStarted
    }

    public var body: some View {
        VStack(spacing: 20) {
            Spacer(minLength: 16)

            VStack(spacing: 14) {
                FabushiAvatarBadge(
                    peerId: Int64(bitPattern: profile.bot.botId),
                    title: profile.bot.displayName,
                    isOnline: false,
                    isBot: true
                )
                .scaleEffect(1.35)
                .padding(.bottom, 6)

                Text(profile.bot.displayName)
                    .fabushiFont(.title2)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)

                if !profile.bot.username.isEmpty {
                    Text("@\(profile.bot.username)")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)
                }

                Divider().overlay(theme.separatorHairline)

                Text(theme.messagingString(.prestartBotHeadline))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Text(
                    profile.bot.descriptionText?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
                        ? profile.bot.descriptionText!
                        : (profile.bot.about ?? theme.messagingString(.prestartBotFallbackDescription))
                )
                .fabushiFont(.body)
                .foregroundStyle(theme.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)

                if let appId = profile.mainAppId, !appId.isEmpty {
                    FabushiGlassPillButton(
                        title: theme.messagingString(.prestartBotOpenMiniAppAction),
                        systemImage: "square.grid.2x2",
                        accessibilityID: FabushiAccessibilityIDs.Inspector.openAppButton,
                        isProminent: false
                    ) {
                        router.navigate(to: .miniApp(id: appId), coordinator: coordinator)
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: 420)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius)
            .padding(.horizontal, 20)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.botDescriptionCard)

            Spacer(minLength: 16)

            VStack(spacing: 0) {
                Divider().overlay(theme.separatorHairline)
                Button {
                    guard !isStarting else { return }
                    isStarting = true
                    Task { @MainActor in
                        let chatId = await store.startPrestartBot(
                            botId: profile.bot.botId,
                            targetKey: profile.bot.username.isEmpty ? String(profile.bot.botId) : profile.bot.username,
                            router: router,
                            locale: theme.locale
                        )
                        isStarting = false
                        if let chatId {
                            onStarted?(chatId)
                        }
                    }
                } label: {
                    HStack(spacing: 8) {
                        if isStarting {
                            ProgressView().controlSize(.small)
                        }
                        Text(theme.messagingString(.prestartBotStartAction))
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.accentForegroundColor)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(isStarting)
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.botStartButton)
            }
            .background(theme.surfaceComposer)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.botStartBar)
        }
        .background(theme.surfacePrimary)
    }
}
