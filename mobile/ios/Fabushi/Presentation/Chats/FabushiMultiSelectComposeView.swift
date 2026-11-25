import SwiftUI

// MARK: - Apple Messages-Style Multi-Select Compose Stage (`#9`, `D10-06`, `D14-05`)

/// Inline multi-select compose stage allowing users to select one or more humans and/or bots,
/// optionally name a group, resolve `@username` handles, and atomically create the conversation.
///
/// Layout & concurrency invariants:
/// - NEVER nests a second `NavigationStack` (embedded directly inside the shell's stage `NavigationStack`).
/// - All in-flight `chats.create` responses are strictly fenced by `store.composeEpoch` & `accountScopeGeneration`.
@MainActor
public struct FabushiMultiSelectComposeView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @FocusState private var isToFieldFocused: Bool

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

    private var isGroupCreation: Bool {
        store.selectedComposeRecipients.count > 1
            || !store.composeGroupTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    public var body: some View {
        @Bindable var bindableStore = store
        let candidates = store.composeCandidates()
        let combinedCandidates = candidates.humans + candidates.bots

        VStack(spacing: 0) {
            composeHeaderBar
            Divider().overlay(theme.separatorHairline)
            recipientInputBar(bindableStore: bindableStore)

            if store.selectedComposeRecipients.count >= 2 || !store.composeGroupTitle.isEmpty {
                groupTitleRow(bindableStore: bindableStore)
            }

            if let errorMessage = store.composeErrorMessage {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.destructiveColor)
                        .accessibilityHidden(true)
                    Text(errorMessage)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.destructiveColor)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(theme.destructiveColor.opacity(0.08))
            }

            Divider().overlay(theme.separatorHairline)

            candidatesListView(
                humans: candidates.humans,
                bots: candidates.bots,
                combined: combinedCandidates
            )
        }
        .background(theme.surfacePrimary)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.stage)
        .onAppear {
            store.beginComposeMode()
            isToFieldFocused = true
        }
        .task(id: coordinator.accountScopeGeneration) {
            if store.contacts.isEmpty && store.bots.isEmpty {
                await store.refreshContactsAndBlocklist()
            }
        }
    }

    // MARK: - Inline Stage Header (Cancel + Title + Create CTA)

    private var composeHeaderBar: some View {
        HStack(spacing: 12) {
            Button {
                store.cancelComposeMode()
                router.navigateBackToChatList(coordinator: coordinator)
            } label: {
                Text(theme.localized("hig.common.cancel"))
                    .fabushiFont(.callout)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.cancelButton)

            VStack(spacing: 2) {
                Text(theme.messagingString(.composeTitle))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                if !store.selectedComposeRecipients.isEmpty {
                    Text(theme.messagingFormat(.composeSelectedCountFormat, store.selectedComposeRecipients.count))
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textSecondary)
                }
            }
            .frame(maxWidth: .infinity)

            Button {
                Task { @MainActor in
                    _ = await store.createComposeConversation(router: router, locale: theme.locale)
                }
            } label: {
                Group {
                    if store.isCreatingComposeChat {
                        ProgressView()
                            .controlSize(.small)
                    } else {
                        Text(
                            isGroupCreation
                                ? theme.messagingString(.composeCreateGroupAction)
                                : theme.messagingString(.composeCreateDirectAction)
                        )
                        .fabushiFont(.callout)
                        .fontWeight(.semibold)
                    }
                }
                .foregroundStyle(
                    store.selectedComposeRecipients.isEmpty
                        ? theme.textMuted
                        : theme.accentForegroundColor
                )
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(
                    store.selectedComposeRecipients.isEmpty
                        ? theme.surfaceInput
                        : theme.accentColor,
                    in: Capsule()
                )
                .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .disabled(store.selectedComposeRecipients.isEmpty || store.isCreatingComposeChat)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.createButton)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(theme.surfaceTopBar)
    }

    // MARK: - `To:` Field & Selected Chips

    private func recipientInputBar(bindableStore: Bindable<FabushiMessagingStore>) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if !store.selectedComposeRecipients.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(store.selectedComposeRecipients) { recipient in
                            HStack(spacing: 6) {
                                if recipient.isBot {
                                    Image(systemName: "sparkles")
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(theme.accentColor)
                                        .accessibilityHidden(true)
                                }
                                Text(recipient.displayName)
                                    .fabushiFont(.subheadline)
                                    .foregroundStyle(theme.textPrimary)
                                    .lineLimit(1)

                                Button {
                                    store.removeComposeRecipient(userId: recipient.userId)
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .fabushiFont(.footnote)
                                        .foregroundStyle(theme.textSecondary)
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel("\(theme.localized("hig.common.close")) \(recipient.displayName)")
                                .accessibilityHint(theme.messagingString(.composeRemoveRecipientHint))
                                .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.chipRemove)
                            }
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(theme.accentColor.opacity(0.14), in: Capsule())
                        }
                    }
                    .padding(.horizontal, 16)
                }
                .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.chipList)
            }

            HStack(spacing: 8) {
                Text(theme.messagingString(.composeToLabel))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)

                TextField(
                    theme.messagingString(.composeSearchPlaceholder),
                    text: bindableStore.composeQuery
                )
                .fabushiFont(.body)
                .foregroundStyle(theme.textPrimary)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .focused($isToFieldFocused)
                .onSubmit {
                    Task { @MainActor in
                        await store.resolveComposeUsername()
                    }
                }
                .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.toField)

                if !store.composeQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Button {
                        Task { @MainActor in
                            await store.resolveComposeUsername()
                        }
                    } label: {
                        Image(systemName: "at.circle.fill")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.messagingString(.composeResolveUsernameAction))
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .background(theme.surfacePrimary)
    }

    private func groupTitleRow(bindableStore: Bindable<FabushiMessagingStore>) -> some View {
        VStack(spacing: 0) {
            Divider().overlay(theme.separatorHairline).padding(.leading, 16)
            HStack(spacing: 8) {
                Image(systemName: "person.3.fill")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
                    .accessibilityHidden(true)

                TextField(
                    theme.messagingString(.composeGroupTitlePlaceholder),
                    text: bindableStore.composeGroupTitle
                )
                .fabushiFont(.body)
                .foregroundStyle(theme.textPrimary)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.groupTitleField)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
    }

    // MARK: - Candidate List

    private func candidatesListView(
        humans: [FabushiComposeRecipient],
        bots: [FabushiComposeRecipient],
        combined: [FabushiComposeRecipient]
    ) -> some View {
        let indexMap = Dictionary(uniqueKeysWithValues: combined.enumerated().map { ($0.element.id, $0.offset) })

        return List {
            if let resolved = store.resolvedComposePeer {
                Section {
                    let recipient = FabushiComposeRecipient(
                        userId: resolved.userId,
                        displayName: resolved.displayName,
                        username: resolved.username,
                        isBot: resolved.isBot,
                        subtitle: resolved.username.map { "@\($0)" } ?? resolved.bio
                    )
                    candidateRow(recipient: recipient, accessibilityID: "fabushi.compose.candidate.resolved")
                } header: {
                    FabushiSectionHeader(theme.messagingString(.composeUsernameLookupSection))
                }
            }

            if !bots.isEmpty {
                Section {
                    ForEach(bots) { bot in
                        let idx = indexMap[bot.id] ?? 0
                        candidateRow(recipient: bot, accessibilityID: candidateAccessibilityID(for: idx))
                    }
                } header: {
                    FabushiSectionHeader(theme.messagingString(.composeBotsSection))
                }
            }

            if !humans.isEmpty {
                Section {
                    ForEach(humans) { human in
                        let idx = indexMap[human.id] ?? 0
                        candidateRow(recipient: human, accessibilityID: candidateAccessibilityID(for: idx))
                    }
                } header: {
                    FabushiSectionHeader(theme.messagingString(.composeHumansSection))
                }
            }

            if humans.isEmpty && bots.isEmpty && store.resolvedComposePeer == nil {
                FabushiEmptyStateView(
                    title: theme.messagingString(.composeNoCandidates),
                    systemImage: "person.crop.circle.badge.questionmark",
                    descriptionText: theme.messagingString(.composeSearchPlaceholder),
                    accessibilityID: "fabushi.compose.emptyCandidates"
                )
                .listRowBackground(theme.surfacePrimary)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Compose.candidateList)
    }

    private func candidateRow(recipient: FabushiComposeRecipient, accessibilityID: String) -> some View {
        let isSelected = store.selectedComposeRecipients.contains { $0.userId == recipient.userId }
        return Button {
            store.toggleComposeRecipient(recipient)
        } label: {
            HStack(spacing: 12) {
                FabushiAvatarBadge(
                    peerId: Int64(bitPattern: recipient.userId),
                    title: recipient.displayName,
                    isOnline: false,
                    isBot: recipient.isBot
                )

                VStack(alignment: .leading, spacing: 2) {
                    Text(recipient.displayName)
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(1)
                    if let subtitle = recipient.subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .lineLimit(1)
                    }
                }

                Spacer(minLength: 8)

                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                    .fabushiFont(.title3)
                    .foregroundStyle(isSelected ? theme.accentColor : theme.textMuted)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fabushiMinimumTouchTarget()
        .listRowBackground(theme.surfacePrimary)
        .accessibilityAddTraits(isSelected ? [.isSelected] : [])
        .accessibilityIdentifier(accessibilityID)
    }

    private func candidateAccessibilityID(for index: Int) -> String {
        switch index {
        case 0: return FabushiAccessibilityIDs.Compose.firstCandidate
        case 1: return FabushiAccessibilityIDs.Compose.secondCandidate
        default: return "fabushi.compose.candidate.\(index)"
        }
    }
}
