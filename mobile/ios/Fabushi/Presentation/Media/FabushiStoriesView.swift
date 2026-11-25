import Foundation
import PhotosUI
import SwiftUI

// MARK: - Stories Viewer & Stories Composer (`#20`, Reduce-Motion-Safe, Real `stories.*` RPCs)

@MainActor
public struct FabushiStoriesView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    @State private var currentIndex: Int = 0
    @State private var segmentProgress: Double = 0
    @State private var isPaused: Bool = false
    @State private var replyText: String = ""
    @State private var isComposerPresented: Bool = false
    @State private var tickerTask: Task<Void, Never>? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var reduceMotion: Bool {
        theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)
    }

    private var currentStory: FabushiStoryItem? {
        guard !store.stories.isEmpty else { return nil }
        let safeIndex = min(max(0, currentIndex), store.stories.count - 1)
        return store.stories[safeIndex]
    }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil,
        onDismiss: @escaping () -> Void = {}
    ) {
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
        self.onDismiss = onDismiss
    }

    public var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()

                if let story = currentStory {
                    storyStageView(story)
                } else {
                    VStack(spacing: 16) {
                        FabushiEmptyStateView(
                            title: theme.messagingString(.storiesTitle),
                            systemImage: "circle.dashed.inset.filled",
                            descriptionText: theme.messagingString(.storiesEmptyTitle),
                            accessibilityID: "fabushi.stories.empty"
                        )
                        FabushiGlassPillButton(
                            title: theme.messagingString(.storiesCreateAction),
                            systemImage: "plus.circle.fill",
                            accessibilityID: "fabushi.stories.createButton",
                            isProminent: true
                        ) {
                            isComposerPresented = true
                        }
                    }
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        stopTicker()
                        store.overlayController.dismissStories()
                        onDismiss()
                    }
                    .foregroundStyle(.white)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        isPaused = true
                        isComposerPresented = true
                    } label: {
                        Image(systemName: "plus")
                            .fabushiFont(.body)
                            .foregroundStyle(.white)
                            .fabushiMinimumTouchTarget()
                    }
                    .accessibilityLabel(theme.messagingString(.storiesCreateAction))
                }
            }
            .task {
                if let initial = store.overlayController.presentedStoryIndex {
                    currentIndex = initial
                }
                await store.refreshStories()
                startTickerIfNeeded()
            }
            .onDisappear {
                stopTicker()
            }
            .sheet(isPresented: $isComposerPresented, onDismiss: {
                isPaused = false
                startTickerIfNeeded()
            }) {
                FabushiStoryComposerSheetView(
                    store: store,
                    onCreated: {
                        currentIndex = 0
                        segmentProgress = 0
                        isComposerPresented = false
                    },
                    onCancel: {
                        isComposerPresented = false
                    }
                )
            }
        }
    }

    private func storyStageView(_ story: FabushiStoryItem) -> some View {
        VStack(spacing: 12) {
            // Segmented progress bars
            HStack(spacing: 4) {
                ForEach(0..<store.stories.count, id: \.self) { idx in
                    GeometryReader { proxy in
                        ZStack(alignment: .leading) {
                            Capsule().fill(Color.white.opacity(0.28))
                            Capsule()
                                .fill(Color.white)
                                .frame(width: proxy.size.width * barFillFraction(for: idx))
                        }
                    }
                    .frame(height: 3)
                }
            }
            .padding(.horizontal, 12)
            .padding(.top, 8)

            // Author header
            HStack(spacing: 10) {
                FabushiAvatarBadge(
                    peerId: Int64(bitPattern: story.userId),
                    title: story.firstName.isEmpty ? (story.username ?? "User") : story.firstName,
                    isOnline: false,
                    isBot: false
                )
                VStack(alignment: .leading, spacing: 2) {
                    Text(story.firstName.isEmpty ? (story.username ?? "User \(story.userId)") : story.firstName)
                        .fabushiFont(.headline)
                        .foregroundStyle(.white)
                    Text(theme.messagingFormat(.storiesViewsCountFormat, story.viewsCount))
                        .fabushiFont(.caption)
                        .foregroundStyle(.white.opacity(0.78))
                }
                Spacer()

                if isOwnStory(story) {
                    Button(role: .destructive) {
                        Task { @MainActor in
                            _ = await store.deleteStory(storyId: story.storyId)
                            if currentIndex >= store.stories.count {
                                currentIndex = max(0, store.stories.count - 1)
                            }
                            segmentProgress = 0
                        }
                    } label: {
                        Image(systemName: "trash")
                            .fabushiFont(.body)
                            .foregroundStyle(.white)
                            .fabushiMinimumTouchTarget()
                    }
                    .accessibilityLabel(theme.messagingString(.deleteChatAction))
                }
            }
            .padding(.horizontal, 16)

            // Story visual surface with tap left/right navigation and hold-to-pause
            ZStack {
                LinearGradient(
                    colors: [
                        Color(fabushiHex: FabushiPeerColorPalette.avatarGradientRGB(peerId: Int64(bitPattern: story.userId)).top),
                        Color(fabushiHex: FabushiPeerColorPalette.avatarGradientRGB(peerId: Int64(bitPattern: story.userId)).bottom),
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
                .clipShape(RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous))

                VStack(spacing: 16) {
                    Image(systemName: story.mediaType == "video" ? "video.fill" : "sparkles")
                        .fabushiFont(.largeTitle)
                        .foregroundStyle(.white.opacity(0.9))
                    if let caption = story.caption, !caption.isEmpty {
                        Text(caption)
                            .fabushiFont(.title3)
                            .foregroundStyle(.white)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 24)
                    }
                }

                HStack(spacing: 0) {
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture { stepStory(delta: -1) }
                    Color.clear
                        .contentShape(Rectangle())
                        .onTapGesture { stepStory(delta: 1) }
                }
            }
            .padding(.horizontal, 12)
            .onLongPressGesture(minimumDuration: 0.25, pressing: { pressing in
                isPaused = pressing
            }, perform: {})

            // Quick reply bar (`D10-07`: sends via `sendNonComposerMessage` preserving unsent composer draft!)
            HStack(spacing: 10) {
                TextField(theme.messagingString(.storiesReplyPlaceholder), text: $replyText)
                    .fabushiFont(.body)
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(Color.white.opacity(0.16), in: Capsule())

                Button {
                    let trimmed = replyText.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !trimmed.isEmpty else { return }
                    replyText = ""
                    let targetChatId = Int64(bitPattern: story.userId)
                    Task { @MainActor in
                        _ = try? await store.sendNonComposerMessage(
                            chatId: targetChatId,
                            text: trimmed
                        )
                    }
                } label: {
                    Image(systemName: "paperplane.fill")
                        .fabushiFont(.body)
                        .foregroundStyle(.white)
                        .padding(10)
                        .background(theme.accentColor, in: Circle())
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(replyText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .accessibilityLabel(theme.messagingString(.composerSendAction))
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 12)
        }
    }

    private func barFillFraction(for idx: Int) -> CGFloat {
        if idx < currentIndex { return 1.0 }
        if idx > currentIndex { return 0.0 }
        return reduceMotion ? 1.0 : CGFloat(min(1.0, max(0.0, segmentProgress)))
    }

    private func isOwnStory(_ story: FabushiStoryItem) -> Bool {
        if case .authenticated(let uid, _, _) = coordinator.authState {
            return story.userId == uid
        }
        return false
    }

    private func stepStory(delta: Int) {
        guard !store.stories.isEmpty else { return }
        let next = currentIndex + delta
        if next < 0 {
            currentIndex = 0
            segmentProgress = 0
        } else if next >= store.stories.count {
            stopTicker()
            store.overlayController.dismissStories()
            onDismiss()
        } else {
            currentIndex = next
            segmentProgress = 0
        }
    }

    private func startTickerIfNeeded() {
        stopTicker()
        guard !reduceMotion, !store.stories.isEmpty else { return }
        tickerTask = Task { @MainActor in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 100_000_000)
                guard !Task.isCancelled else { break }
                if isPaused || store.stories.isEmpty { continue }
                segmentProgress += 0.02
                if segmentProgress >= 1.0 {
                    stepStory(delta: 1)
                }
            }
        }
    }

    private func stopTicker() {
        tickerTask?.cancel()
        tickerTask = nil
    }
}

// MARK: - Stories Composer Sheet

@MainActor
private struct FabushiStoryComposerSheetView: View {
    let store: FabushiMessagingStore
    let onCreated: () -> Void
    let onCancel: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var caption: String = ""
    @State private var privacy: String = "everyone"
    @State private var selectedPhotoItem: PhotosPickerItem? = nil
    @State private var selectedPhotoData: Data? = nil
    @State private var isPublishing: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    let pickerTitle = theme.messagingString(.attachPhotoVideoAction)
                    let pickerIcon = selectedPhotoData == nil ? "photo.badge.plus" : "checkmark.circle.fill"
                    PhotosPicker(
                        selection: $selectedPhotoItem,
                        matching: .images
                    ) {
                        Label(pickerTitle, systemImage: pickerIcon)
                            .font(FabushiTextStyle.body.font)
                    }
                    .onChange(of: selectedPhotoItem) { _, newItem in
                        guard let newItem else { return }
                        Task { @MainActor in
                            selectedPhotoData = try? await newItem.loadTransferable(type: Data.self)
                        }
                    }

                    TextField(theme.messagingString(.storiesCaptionPlaceholder), text: $caption, axis: .vertical)
                        .fabushiFont(.body)
                        .lineLimit(3...6)
                }

                Section {
                    Picker(theme.messagingString(.storiesPrivacyLabel), selection: $privacy) {
                        Text("Everyone").tag("everyone")
                        Text("Contacts").tag("contacts")
                    }
                }
            }
            .navigationTitle(theme.messagingString(.storiesComposerTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.storiesPublishAction)) {
                        guard !isPublishing else { return }
                        isPublishing = true
                        Task { @MainActor in
                            var uploadedMediaId: String? = nil
                            if let data = selectedPhotoData {
                                let res = try? await store.client.uploadMedia(
                                    filename: "story-\(UUID().uuidString).jpg",
                                    mimeType: "image/jpeg",
                                    dataB64: data.base64EncodedString()
                                )
                                uploadedMediaId = res?.fileId
                            }
                            let trimmedCaption = caption.trimmingCharacters(in: .whitespacesAndNewlines)
                            _ = await store.createStory(
                                mediaId: uploadedMediaId,
                                mediaType: "photo",
                                caption: trimmedCaption.isEmpty ? nil : trimmedCaption,
                                privacy: privacy
                            )
                            isPublishing = false
                            onCreated()
                        }
                    }
                    .disabled(
                        isPublishing
                            || (caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && selectedPhotoData == nil)
                    )
                }
            }
        }
    }
}
