import Foundation
import SwiftUI

// MARK: - Stickers, Emoji & GIFs Picker (`#20`, `D10-07` Non-Composer Draft-Preserving Sends)

public enum FabushiStickerPickerTab: String, CaseIterable, Sendable, Identifiable {
    case emoji
    case stickers
    case gifs

    public var id: String { rawValue }
}

@MainActor
public struct FabushiStickersAndGifsPickerView: View {
    public let chatId: Int64
    public let store: FabushiMessagingStore
    public let onInsertEmoji: (String) -> Void
    public let onSentNonComposerMedia: () -> Void
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var selectedTab: FabushiStickerPickerTab = .emoji
    @State private var stickerPacks: [FabushiStickerPack] = []
    @State private var gifs: [FabushiGifItem] = []
    @State private var gifQuery: String = ""
    @State private var isLoadingStickers: Bool = false
    @State private var isLoadingGifs: Bool = false
    @State private var gifSearchTask: Task<Void, Never>? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public static let curatedEmojiCategories: [(title: String, emojis: [String])] = [
        ("Smileys", ["😀", "😂", "🥹", "😊", "😍", "🤔", "🤩", "😎", "🥳", "😅", "🙏", "👍", "👎", "🔥", "❤️", "🎉"]),
        ("Work", ["💻", "🚀", "⚡️", "🛠️", "📦", "📝", "📌", "🔍", "✅", "⚠️", "🔒", "🤖", "🧠", "📊", "🌐", "⏱️"]),
        ("Nature", ["☀️", "🌙", "⭐️", "🌈", "🌸", "🌿", "🍀", "🌊", "🍎", "☕️", "🍵", "🎵", "🎯", "🏆", "💡", "✨"]),
    ]

    public init(
        chatId: Int64,
        store: FabushiMessagingStore,
        onInsertEmoji: @escaping (String) -> Void,
        onSentNonComposerMedia: @escaping () -> Void = {},
        onDismiss: @escaping () -> Void
    ) {
        self.chatId = chatId
        self.store = store
        self.onInsertEmoji = onInsertEmoji
        self.onSentNonComposerMedia = onSentNonComposerMedia
        self.onDismiss = onDismiss
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Picker(theme.messagingString(.stickerPickerTitle), selection: $selectedTab) {
                    Text(theme.messagingString(.stickerTabEmoji)).tag(FabushiStickerPickerTab.emoji)
                    Text(theme.messagingString(.stickerTabStickers)).tag(FabushiStickerPickerTab.stickers)
                    Text(theme.messagingString(.stickerTabGifs)).tag(FabushiStickerPickerTab.gifs)
                }
                .pickerStyle(.segmented)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)

                Divider().overlay(theme.separatorHairline)

                switch selectedTab {
                case .emoji:
                    emojiGridView
                case .stickers:
                    stickersView
                case .gifs:
                    gifsView
                }
            }
            .background(theme.surfacePrimary)
            .navigationTitle(theme.messagingString(.stickerPickerTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close"), action: onDismiss)
                }
            }
            .task {
                await loadStickerPacks()
            }
            .onDisappear {
                gifSearchTask?.cancel()
            }
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.stickerGrid)
    }

    private var emojiGridView: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                ForEach(Self.curatedEmojiCategories, id: \.title) { category in
                    FabushiSectionHeader(category.title)
                        .padding(.horizontal, 16)

                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 8), spacing: 8) {
                        ForEach(category.emojis, id: \.self) { emoji in
                            Button {
                                onInsertEmoji(emoji)
                            } label: {
                                Text(emoji)
                                    .fabushiFont(.title2)
                                    .frame(maxWidth: .infinity)
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(emoji)
                        }
                    }
                    .padding(.horizontal, 12)
                }
            }
            .padding(.vertical, 12)
        }
    }

    private var stickersView: some View {
        Group {
            if isLoadingStickers && stickerPacks.isEmpty {
                VStack {
                    Spacer()
                    ProgressView()
                    Spacer()
                }
            } else if stickerPacks.isEmpty {
                FabushiEmptyStateView(
                    title: theme.messagingString(.stickerTabStickers),
                    systemImage: "face.smiling",
                    descriptionText: theme.messagingString(.inspectorNoSharedMedia),
                    accessibilityID: "fabushi.stickers.empty"
                )
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        ForEach(stickerPacks) { pack in
                            VStack(alignment: .leading, spacing: 10) {
                                HStack {
                                    Text(pack.title.isEmpty ? pack.shortName : pack.title)
                                        .fabushiFont(.headline)
                                        .foregroundStyle(theme.textPrimary)
                                    Spacer()
                                    Button {
                                        Task { @MainActor in
                                            await toggleInstallPack(pack)
                                        }
                                    } label: {
                                        Text(
                                            pack.isInstalled
                                                ? theme.messagingString(.stickerInstalledBadge)
                                                : theme.messagingString(.stickerInstallAction)
                                        )
                                        .fabushiFont(.caption)
                                        .fontWeight(.semibold)
                                        .foregroundStyle(pack.isInstalled ? theme.textSecondary : theme.accentForegroundColor)
                                        .padding(.horizontal, 12)
                                        .padding(.vertical, 6)
                                        .background(
                                            pack.isInstalled ? theme.surfaceInput : theme.accentColor,
                                            in: Capsule()
                                        )
                                        .fabushiMinimumTouchTarget()
                                    }
                                    .buttonStyle(.plain)
                                }
                                .padding(.horizontal, 16)

                                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 4), spacing: 10) {
                                    ForEach(pack.stickers) { sticker in
                                        Button {
                                            Task { @MainActor in
                                                await sendStickerPreservingDraft(sticker)
                                            }
                                        } label: {
                                            VStack(spacing: 4) {
                                                Text(sticker.emoji.isEmpty ? "✨" : sticker.emoji)
                                                    .fabushiFont(.largeTitle)
                                                Text(pack.shortName)
                                                    .fabushiFont(.caption2)
                                                    .foregroundStyle(theme.textMuted)
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
                                        .accessibilityLabel(sticker.emoji.isEmpty ? pack.title : sticker.emoji)
                                    }
                                }
                                .padding(.horizontal, 16)
                            }
                        }
                    }
                    .padding(.vertical, 12)
                }
            }
        }
    }

    private var gifsView: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textMuted)
                TextField(theme.messagingString(.gifSearchPlaceholder), text: $gifQuery)
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textPrimary)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .onChange(of: gifQuery) { _, newValue in
                        scheduleGifSearch(query: newValue)
                    }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                theme.surfaceInput,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous)
            )
            .padding(.horizontal, 16)
            .padding(.vertical, 10)

            if isLoadingGifs && gifs.isEmpty {
                Spacer()
                ProgressView()
                Spacer()
            } else if gifs.isEmpty {
                FabushiEmptyStateView(
                    title: theme.messagingString(.stickerTabGifs),
                    systemImage: "sparkles.rectangle.stack",
                    descriptionText: theme.messagingString(.gifSearchPlaceholder),
                    accessibilityID: "fabushi.gifs.empty"
                )
            } else {
                ScrollView {
                    LazyVGrid(columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)], spacing: 10) {
                        ForEach(gifs) { gif in
                            Button {
                                Task { @MainActor in
                                    await sendGifPreservingDraft(gif)
                                }
                            } label: {
                                VStack(alignment: .leading, spacing: 6) {
                                    ZStack {
                                        RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                                            .fill(theme.surfaceElevated)
                                            .frame(height: 92)
                                        VStack(spacing: 4) {
                                            Image(systemName: "play.square.stack")
                                                .fabushiFont(.title2)
                                                .foregroundStyle(theme.accentColor)
                                            Text("GIF")
                                                .fabushiFont(.caption2)
                                                .fontWeight(.bold)
                                                .foregroundStyle(theme.textSecondary)
                                        }
                                    }
                                    Text(gif.title.isEmpty ? gif.id : gif.title)
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.textPrimary)
                                        .lineLimit(1)
                                }
                                .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(16)
                }
            }
        }
        .task {
            if gifs.isEmpty {
                await searchGifsNow(query: gifQuery.isEmpty ? "wave" : gifQuery)
            }
        }
    }

    private func loadStickerPacks() async {
        isLoadingStickers = true
        defer { isLoadingStickers = false }
        if let fetched = try? await store.client.listStickerSets() {
            stickerPacks = fetched
        }
    }

    private func toggleInstallPack(_ pack: FabushiStickerPack) async {
        let targetState = !pack.isInstalled
        do {
            try await store.client.installStickerSet(setId: pack.setId, installed: targetState)
            if let idx = stickerPacks.firstIndex(where: { $0.setId == pack.setId }) {
                stickerPacks[idx].isInstalled = targetState
            }
        } catch {
            // Keep authoritative installed state on RPC failure.
        }
    }

    private func scheduleGifSearch(query: String) {
        gifSearchTask?.cancel()
        gifSearchTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 220_000_000)
            guard !Task.isCancelled else { return }
            let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
            await searchGifsNow(query: trimmed.isEmpty ? "wave" : trimmed)
        }
    }

    private func searchGifsNow(query: String) async {
        isLoadingGifs = true
        defer { isLoadingGifs = false }
        if let fetched = try? await store.client.searchGifs(query: query, limit: 24) {
            gifs = fetched
        }
    }

    /// `D10-07`: Uses `store.sendNonComposerMessage(...)` so unsent composer draft is untouched!
    private func sendStickerPreservingDraft(_ sticker: FabushiStickerItem) async {
        let payloadJSON = """
        {"kind":"sticker","file_id":"\(sticker.fileId)","emoji":"\(sticker.emoji)"}
        """
        _ = try? await store.sendNonComposerMessage(
            chatId: chatId,
            text: sticker.emoji.isEmpty ? nil : sticker.emoji,
            content: .extended(kind: "sticker", rawJSON: payloadJSON)
        )
        onSentNonComposerMedia()
        onDismiss()
    }

    /// `D10-07`: Uses `store.sendNonComposerMessage(...)` so unsent composer draft is untouched!
    private func sendGifPreservingDraft(_ gif: FabushiGifItem) async {
        let media = FabushiMediaItemPayload(
            fileId: gif.fileId,
            fileName: "\(gif.id).gif",
            mimeType: gif.mimeType.isEmpty ? "image/gif" : gif.mimeType,
            sizeBytes: 0,
            width: gif.width,
            height: gif.height,
            caption: gif.title.isEmpty ? nil : gif.title
        )
        _ = try? await store.sendNonComposerMessage(
            chatId: chatId,
            text: gif.title.isEmpty ? nil : gif.title,
            content: .media(media)
        )
        onSentNonComposerMedia()
        onDismiss()
    }
}
