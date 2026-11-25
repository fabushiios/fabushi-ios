import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - 900-Second Same-Sender Bubble Grouping & Date Separators (`#13`)

public enum FabushiBubbleClusterPosition: Sendable, Equatable {
    case single
    case first
    case middle
    case last
}

public struct FabushiClusteredMessageItem: Identifiable, Sendable, Equatable {
    public var id: Int64 { message.messageId }

    public let message: FabushiMessageRecord
    public let position: FabushiBubbleClusterPosition
    public let showsTail: Bool
    public let showsSenderHeader: Bool
    public let showsDateSeparator: Bool
    public let dateSeparatorLabel: String

    public init(
        message: FabushiMessageRecord,
        position: FabushiBubbleClusterPosition,
        showsTail: Bool,
        showsSenderHeader: Bool,
        showsDateSeparator: Bool,
        dateSeparatorLabel: String
    ) {
        self.message = message
        self.position = position
        self.showsTail = showsTail
        self.showsSenderHeader = showsSenderHeader
        self.showsDateSeparator = showsDateSeparator
        self.dateSeparatorLabel = dateSeparatorLabel
    }
}

public enum FabushiMessageClusterCalculator: Sendable {
    /// Upstream `v1.6.15` canonical same-sender grouping window: 900 seconds (15 minutes).
    public static let clusterIntervalMs: Int64 = 900_000

    public static func compute(
        messages: [FabushiMessageRecord],
        isGroupChat: Bool,
        calendar: Calendar = .current,
        locale: Locale = .current
    ) -> [FabushiClusteredMessageItem] {
        guard !messages.isEmpty else { return [] }
        var result: [FabushiClusteredMessageItem] = []
        result.reserveCapacity(messages.count)

        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.dateStyle = .medium
        formatter.timeStyle = .none

        for idx in messages.indices {
            let current = messages[idx]
            let previous = idx > 0 ? messages[idx - 1] : nil
            let next = idx + 1 < messages.count ? messages[idx + 1] : nil

            let currentDate = Date(timeIntervalSince1970: Double(current.dateMs) / 1000.0)
            let previousDate = previous.map { Date(timeIntervalSince1970: Double($0.dateMs) / 1000.0) }

            let sameDayAsPrev = previousDate.map { calendar.isDate(currentDate, inSameDayAs: $0) } ?? false
            let showsDateSeparator = !sameDayAsPrev
            let dateSeparatorLabel = showsDateSeparator ? formatter.string(from: currentDate) : ""

            let isService = isServiceCard(current)
            let groupedWithPrev = !isService
                && previous != nil
                && !isServiceCard(previous!)
                && sameDayAsPrev
                && previous!.senderId == current.senderId
                && previous!.isOutgoing == current.isOutgoing
                && abs(current.dateMs - previous!.dateMs) <= clusterIntervalMs

            let groupedWithNext: Bool
            if let next, !isService, !isServiceCard(next) {
                let nextDate = Date(timeIntervalSince1970: Double(next.dateMs) / 1000.0)
                let sameDayAsNext = calendar.isDate(currentDate, inSameDayAs: nextDate)
                groupedWithNext = sameDayAsNext
                    && next.senderId == current.senderId
                    && next.isOutgoing == current.isOutgoing
                    && abs(next.dateMs - current.dateMs) <= clusterIntervalMs
            } else {
                groupedWithNext = false
            }

            let position: FabushiBubbleClusterPosition
            switch (groupedWithPrev, groupedWithNext) {
            case (false, false): position = .single
            case (false, true): position = .first
            case (true, true): position = .middle
            case (true, false): position = .last
            }

            let showsTail = !isService && (position == .single || position == .last)
            let showsSenderHeader = !isService && isGroupChat && !current.isOutgoing && (position == .single || position == .first)

            result.append(
                FabushiClusteredMessageItem(
                    message: current,
                    position: position,
                    showsTail: showsTail,
                    showsSenderHeader: showsSenderHeader,
                    showsDateSeparator: showsDateSeparator,
                    dateSeparatorLabel: dateSeparatorLabel
                )
            )
        }
        return result
    }

    public static func isServiceCard(_ message: FabushiMessageRecord) -> Bool {
        if case .extended(let kind, _)? = message.content {
            return kind.lowercased() == "service"
        }
        return false
    }
}

// MARK: - Canonical Card Classification (All 11+ Card Types)

public enum FabushiCanonicalCardKind: String, CaseIterable, Sendable, Equatable {
    case text
    case photo
    case video
    case voice
    case file
    case sticker
    case album
    case location
    case poll
    case todo
    case service
    case call
    case storyMention
    case invoice
    case generative
    case codingDiff
    case browserTakeover
    case distilledScript
    case computer
    case encrypted

    public static func classify(_ message: FabushiMessageRecord) -> FabushiCanonicalCardKind {
        guard let content = message.content else {
            return .text
        }
        switch content {
        case .text:
            if message.inlineKeyboard.flatMap({ $0 }).contains(where: { ($0.callbackData ?? "").hasPrefix("browser_") }) {
                return .browserTakeover
            }
            if message.inlineKeyboard.flatMap({ $0 }).contains(where: { ($0.callbackData ?? "").hasPrefix("computer:") }) {
                return .computer
            }
            return .text
        case .photo:
            return .photo
        case .video, .videoNote:
            return .video
        case .voice, .audio:
            return .voice
        case .document:
            return .file
        case .media(let media):
            if media.mimeType.lowercased().hasPrefix("video/") { return .video }
            if media.mimeType.lowercased().hasPrefix("audio/") { return .voice }
            if media.mimeType.lowercased().hasPrefix("image/") { return .photo }
            return .file
        case .poll:
            return .poll
        case .todo:
            return .todo
        case .invoice:
            return .invoice
        case .generativeCard:
            return .generative
        case .encrypted:
            return .encrypted
        case .extended(let kind, _):
            switch kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
            case "sticker", "animated_sticker": return .sticker
            case "album", "media_group": return .album
            case "location", "geo": return .location
            case "service": return .service
            case "call": return .call
            case "story", "story_mention", "story_reply": return .storyMention
            case "coding_diff", "diff": return .codingDiff
            case "browser_takeover_requested", "browser_takeover": return .browserTakeover
            case "browser_script_distilled", "distilled_script": return .distilledScript
            case "computer", "remote_computer": return .computer
            default: return .text
            }
        }
    }
}

// MARK: - Canonical Message Card Router View (`#13`, `D10-08`, `D11-02`, `D14-04`)

@MainActor
public struct FabushiMessageCardView: View {
    public let item: FabushiClusteredMessageItem
    public let chatId: Int64
    public let isLatest: Bool
    public let isSelectedInMultiSelect: Bool
    public let voicePlayer: FabushiVoicePlayerController
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore
    public let onReplyTap: () -> Void
    public let onReactionTap: (String) -> Void
    public let onRetryFailedTap: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var revealedSpoilers: Bool = false
    @State private var isDownloadingFile: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var message: FabushiMessageRecord { item.message }

    private var bubbleTextColor: Color {
        message.isOutgoing ? theme.bubbleOutgoingText : theme.textPrimary
    }

    private var bubbleSecondaryTextColor: Color {
        message.isOutgoing
            ? Color(fabushiHex: theme.palette.bubbleOutgoingSecondaryTextRGB)
            : theme.textSecondary
    }

    public init(
        item: FabushiClusteredMessageItem,
        chatId: Int64,
        isLatest: Bool,
        isSelectedInMultiSelect: Bool = false,
        voicePlayer: FabushiVoicePlayerController,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore,
        onReplyTap: @escaping () -> Void = {},
        onReactionTap: @escaping (String) -> Void = { _ in },
        onRetryFailedTap: @escaping () -> Void = {}
    ) {
        self.item = item
        self.chatId = chatId
        self.isLatest = isLatest
        self.isSelectedInMultiSelect = isSelectedInMultiSelect
        self.voicePlayer = voicePlayer
        self.coordinator = coordinator
        self.router = router
        self.store = store
        self.onReplyTap = onReplyTap
        self.onReactionTap = onReactionTap
        self.onRetryFailedTap = onRetryFailedTap
    }

    public var body: some View {
        let cardKind = FabushiCanonicalCardKind.classify(message)

        VStack(spacing: 6) {
            if item.showsDateSeparator {
                Text(item.dateSeparatorLabel)
                    .fabushiFont(.caption)
                    .fontWeight(.medium)
                    .foregroundStyle(Color(fabushiHex: theme.palette.servicePillForegroundRGB))
                    .padding(.horizontal, 12)
                    .padding(.vertical, 4)
                    .background(
                        Color(fabushiHex: theme.palette.servicePillBackgroundRGB),
                        in: Capsule()
                    )
                    .padding(.vertical, 6)
            }

            if cardKind == .service {
                servicePillView
            } else {
                HStack(alignment: .bottom, spacing: 8) {
                    if message.isOutgoing {
                        Spacer(minLength: 42)
                    }

                    VStack(alignment: message.isOutgoing ? .trailing : .leading, spacing: 4) {
                        if cardKind == .sticker {
                            stickerCardView
                        } else {
                            bubbleContainerView(cardKind: cardKind)
                        }

                        if !message.inlineKeyboard.isEmpty && !message.isDeletedTombstone {
                            inlineKeyboardView
                        }

                        if !message.reactions.isEmpty && !message.isDeletedTombstone {
                            reactionsRowView
                        }
                    }

                    if !message.isOutgoing {
                        Spacer(minLength: 42)
                    }
                }
                .padding(.horizontal, 12)
                .background(
                    isSelectedInMultiSelect ? theme.accentColor.opacity(0.12) : Color.clear,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )
            }
        }
        .accessibilityIdentifier(
            isLatest
                ? FabushiAccessibilityIDs.Stage.messageBubbleLatest
                : "fabushi.stage.messageBubble.\(message.messageId)"
        )
    }

    // MARK: - Bubble Container

    private func bubbleContainerView(cardKind: FabushiCanonicalCardKind) -> some View {
        let radii = bubbleRadii()

        return VStack(alignment: .leading, spacing: 6) {
            if item.showsSenderHeader {
                Text(senderLabel)
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(
                        Color(fabushiHex: FabushiPeerColorPalette.peerNameRGB(
                            peerId: Int64(bitPattern: message.senderId),
                            preset: theme.preset
                        ))
                    )
            }

            if let forward = message.forwardInfo {
                HStack(spacing: 4) {
                    Image(systemName: "arrowshape.turn.up.right.fill")
                        .fabushiFont(.caption2)
                    Text(forward.fromName ?? theme.messagingString(.messageForwardAction))
                        .fabushiFont(.caption)
                        .fontWeight(.medium)
                }
                .foregroundStyle(bubbleSecondaryTextColor)
            }

            if let reply = message.replyTo, !message.isDeletedTombstone {
                replyHeaderStrip(reply)
            }

            if message.isDeletedTombstone {
                HStack(spacing: 6) {
                    Image(systemName: "nosign")
                        .fabushiFont(.caption)
                    Text(theme.messagingString(.messageDeletedTombstone))
                        .fabushiFont(.callout)
                        .italic()
                }
                .foregroundStyle(bubbleSecondaryTextColor)
            } else {
                cardBodyView(cardKind: cardKind)
            }

            // Timestamp, Edited Badge, TTL & Delivery Status
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                if message.deliveryStatus == .failed {
                    Button(action: onRetryFailedTap) {
                        Label(theme.messagingString(.outboxRetryItemAction), systemImage: "exclamationmark.circle.fill")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.destructiveColor)
                    }
                    .buttonStyle(.plain)
                }
                if message.editDateMs != nil && !message.isDeletedTombstone {
                    Text(theme.messagingString(.messageEditedBadge))
                        .fabushiFont(.caption2)
                        .foregroundStyle(bubbleSecondaryTextColor)
                }
                Text(formattedTime(message.dateMs))
                    .fabushiFont(.caption2)
                    .foregroundStyle(bubbleSecondaryTextColor)
                if message.isOutgoing {
                    deliveryTickIcon
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.messageStatusBadge)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background {
            if message.isOutgoing {
                UnevenRoundedRectangle(cornerRadii: radii, style: .continuous)
                    .fill(theme.bubbleOutgoingGradient)
            } else {
                UnevenRoundedRectangle(cornerRadii: radii, style: .continuous)
                    .fill(theme.bubbleIncoming)
            }
        }
    }

    // MARK: - 11+ Canonical Card Body Dispatcher

    @ViewBuilder
    private func cardBodyView(cardKind: FabushiCanonicalCardKind) -> some View {
        switch cardKind {
        case .text:
            richTextCardView(text: message.text, entities: message.entities)
        case .photo:
            photoCardView
        case .video:
            videoCardView
        case .voice:
            voiceCardView
        case .file:
            fileDocumentCardView
        case .sticker:
            stickerCardView
        case .album:
            albumCardView
        case .location:
            locationCardView
        case .poll:
            if case .poll(let poll)? = message.content {
                pollCardView(poll)
            } else {
                richTextCardView(text: message.text, entities: message.entities)
            }
        case .todo:
            if case .todo(let todo)? = message.content {
                todoCardView(todo)
            } else {
                richTextCardView(text: message.text, entities: message.entities)
            }
        case .service:
            servicePillView
        case .call:
            callBubbleView
        case .storyMention:
            storyMentionCardView
        case .invoice:
            if case .invoice(let invoice)? = message.content {
                invoiceCardView(invoice)
            } else {
                richTextCardView(text: message.text, entities: message.entities)
            }
        case .generative:
            if case .generativeCard(let card)? = message.content {
                generativeCardView(card)
            } else {
                richTextCardView(text: message.text, entities: message.entities)
            }
        case .codingDiff:
            codingDiffCardView
        case .browserTakeover:
            browserTakeoverCardView
        case .distilledScript:
            distilledScriptInlineCardView
        case .computer:
            computerCardView
        case .encrypted:
            encryptedMessageCardView
        }
    }

    // MARK: - 1. Rich Text, Code Blocks (Copy), Math (`SwiftMath`), Mermaid & Spoilers

    private func richTextCardView(text: String, entities: [FabushiMessageEntity]) -> some View {
        let hasSpoiler = entities.contains(where: { $0.entityType == .spoiler })
            || (text.contains("||") && text.components(separatedBy: "||").count >= 3)
        let mermaidSegments = splitMobileAssistantMermaid(text)

        return VStack(alignment: .leading, spacing: 8) {
            if hasSpoiler && !revealedSpoilers {
                Button {
                    revealedSpoilers = true
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "eye.slash.fill")
                            .fabushiFont(.footnote)
                        Text(theme.messagingString(.spoilerTapToReveal))
                            .fabushiFont(.callout)
                    }
                    .foregroundStyle(bubbleTextColor)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(Color.black.opacity(0.18), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
            } else {
                ForEach(mermaidSegments) { segment in
                    if segment.kind == .mermaid {
                        MobileMermaidDiagramView(
                            source: segment.text,
                            renderScopeID: "msg-\(chatId)-\(message.messageId)-\(segment.id)"
                        )
                    } else {
                        renderTextWithCodeAndMath(segment.text, entities: entities)
                    }
                }
            }

            // Instant View preview pill when message contains an HTTP(S) link
            if let firstURL = extractFirstHTTPURL(from: text, entities: entities) {
                Button {
                    let doc = FabushiInstantViewParser.parse(
                        url: firstURL,
                        fallbackTitle: URL(string: firstURL)?.host ?? firstURL,
                        rawText: text
                    )
                    store.overlayController.openInstantView(doc)
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "doc.richtext")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.accentColor)
                        Text(theme.messagingString(.instantViewTitle))
                            .fabushiFont(.caption)
                            .fontWeight(.semibold)
                            .foregroundStyle(theme.accentColor)
                        Spacer(minLength: 4)
                        Image(systemName: "chevron.right")
                            .fabushiFont(.caption2)
                            .foregroundStyle(bubbleSecondaryTextColor)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.surfaceElevated.opacity(0.85), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.instantViewCard)
            }
        }
    }

    @ViewBuilder
    private func renderTextWithCodeAndMath(_ rawText: String, entities: [FabushiMessageEntity]) -> some View {
        let codeSplit = Self.splitFencedCodeBlocks(rawText)
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(codeSplit.enumerated()), id: \.offset) { _, part in
                if part.isCode {
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text((part.language?.isEmpty == false ? part.language! : "code").uppercased())
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                            Spacer()
                            Button {
                                #if canImport(UIKit)
                                UIPasteboard.general.string = part.content
                                #endif
                                router.toastController.showStagePill(
                                    theme.messagingString(.messageCopiedPill),
                                    chatId: chatId
                                )
                            } label: {
                                Label(theme.messagingString(.copyCodeAction), systemImage: "doc.on.doc")
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.accentColor)
                            }
                            .buttonStyle(.plain)
                        }
                        ScrollView(.horizontal, showsIndicators: false) {
                            Text(part.content)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textPrimary)
                                .textSelection(.enabled)
                        }
                    }
                    .padding(10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                } else {
                    let mathSegments = splitMobileAssistantMath(part.content)
                    if mathSegments.contains(where: { $0.kind == .math }) {
                        ForEach(mathSegments) { mathSeg in
                            if mathSeg.kind == .math {
                                MobileMathExpressionView(
                                    expression: mathSeg.text,
                                    displayMode: mathSeg.displayMode
                                )
                            } else if !mathSeg.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                Text(Self.attributedFormattedText(mathSeg.text, entities: entities))
                                    .fabushiFont(.body)
                                    .foregroundStyle(bubbleTextColor)
                                    .textSelection(.enabled)
                            }
                        }
                    } else if !part.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Text(Self.attributedFormattedText(part.content, entities: entities))
                            .fabushiFont(.body)
                            .foregroundStyle(bubbleTextColor)
                            .textSelection(.enabled)
                    }
                }
            }
        }
    }

    // MARK: - 2. Photo & Video Cards

    private var photoCardView: some View {
        let info: (fileId: String, caption: String?, fileName: String) = {
            switch message.content {
            case .photo(let fid, let caption, _, _):
                return (fid, caption, "\(fid).jpg")
            case .media(let media):
                return (media.fileId, media.caption, media.fileName ?? "\(media.fileId).jpg")
            default:
                return ("photo-\(message.messageId)", message.text, "photo.jpg")
            }
        }()

        return VStack(alignment: .leading, spacing: 6) {
            Button {
                store.overlayController.openMediaLightbox(
                    FabushiMediaViewerTarget(
                        fileId: info.fileId,
                        fileName: info.fileName,
                        mimeType: "image/jpeg",
                        caption: info.caption,
                        isVideo: false,
                        chatId: chatId
                    )
                )
            } label: {
                ZStack {
                    RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                        .fill(theme.surfaceElevated)
                        .frame(width: 220, height: 150)
                    VStack(spacing: 6) {
                        Image(systemName: "photo.fill")
                            .fabushiFont(.title2)
                            .foregroundStyle(theme.accentColor)
                        Text(info.fileName)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .lineLimit(1)
                            .padding(.horizontal, 10)
                    }
                }
            }
            .buttonStyle(.plain)

            if let caption = info.caption, !caption.isEmpty {
                Text(caption)
                    .fabushiFont(.body)
                    .foregroundStyle(bubbleTextColor)
            }
        }
    }

    private var videoCardView: some View {
        let info: (fileId: String, duration: UInt32, caption: String?, fileName: String) = {
            switch message.content {
            case .video(let fid, let dur, _, _, let caption):
                return (fid, dur, caption, "\(fid).mp4")
            case .videoNote(let note):
                return (note.fileId, note.durationSeconds, nil, "\(note.fileId).mp4")
            case .media(let media):
                return (media.fileId, media.durationSeconds ?? 0, media.caption, media.fileName ?? "\(media.fileId).mp4")
            default:
                return ("video-\(message.messageId)", 0, message.text, "video.mp4")
            }
        }()

        return VStack(alignment: .leading, spacing: 6) {
            Button {
                store.overlayController.openMediaLightbox(
                    FabushiMediaViewerTarget(
                        fileId: info.fileId,
                        fileName: info.fileName,
                        mimeType: "video/mp4",
                        caption: info.caption,
                        isVideo: true,
                        chatId: chatId
                    )
                )
            } label: {
                ZStack(alignment: .bottomTrailing) {
                    RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                        .fill(Color.black.opacity(0.75))
                        .frame(width: 220, height: 145)
                        .overlay(
                            Image(systemName: "play.circle.fill")
                                .fabushiFont(.largeTitle)
                                .foregroundStyle(.white)
                        )
                    Text(formatDuration(info.duration))
                        .fabushiFont(.caption2)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(Color.black.opacity(0.6), in: Capsule())
                        .padding(8)
                }
            }
            .buttonStyle(.plain)

            if let caption = info.caption, !caption.isEmpty {
                Text(caption)
                    .fabushiFont(.body)
                    .foregroundStyle(bubbleTextColor)
            }
        }
    }

    // MARK: - 3. 5-Bit Waveform Voice & Audio Card

    private var voiceCardView: some View {
        let info: (fileId: String, duration: UInt32, waveform: [UInt8], title: String?) = {
            switch message.content {
            case .voice(let v):
                return (v.fileId, v.durationSeconds, v.waveform, nil)
            case .audio(let fid, let dur, let title, _):
                return (fid, dur, [], title)
            case .media(let media):
                return (media.fileId, media.durationSeconds ?? 0, [], media.fileName)
            default:
                return ("voice-\(message.messageId)", 0, [], nil)
            }
        }()

        let isThisPlaying = voicePlayer.activeFileId == info.fileId && voicePlayer.isPlaying
        let progress = voicePlayer.activeFileId == info.fileId ? voicePlayer.progress : 0.0
        let bars = FabushiVoicePlayerController.normalizedWaveformBars(from: info.waveform)
        let transcript = voicePlayer.transcriptsByFileId[info.fileId]

        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Button {
                    Task { @MainActor in
                        await voicePlayer.togglePlay(
                            fileId: info.fileId,
                            fallbackDurationSeconds: info.duration,
                            chatId: chatId,
                            store: store,
                            router: router,
                            isCompact: horizontalSizeClass == .compact,
                            locale: theme.locale
                        )
                    }
                } label: {
                    Image(systemName: isThisPlaying ? "pause.circle.fill" : "play.circle.fill")
                        .fabushiFont(.title2)
                        .foregroundStyle(message.isOutgoing ? theme.bubbleOutgoingText : theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)

                VStack(alignment: .leading, spacing: 4) {
                    if let title = info.title, !title.isEmpty {
                        Text(title)
                            .fabushiFont(.caption)
                            .fontWeight(.semibold)
                            .foregroundStyle(bubbleTextColor)
                            .lineLimit(1)
                    }
                    FabushiWaveformBarsView(
                        bars: bars,
                        progress: progress,
                        activeTint: message.isOutgoing ? theme.bubbleOutgoingText : theme.accentColor,
                        inactiveTint: bubbleSecondaryTextColor.opacity(0.45),
                        onSeek: { fraction in
                            if voicePlayer.activeFileId == info.fileId {
                                voicePlayer.seek(toProgress: fraction)
                            }
                        }
                    )
                    HStack(spacing: 8) {
                        Text(formatDuration(info.duration))
                            .fabushiFont(.caption2)
                            .monospacedDigit()
                            .foregroundStyle(bubbleSecondaryTextColor)

                        Button {
                            voicePlayer.cyclePlaybackRate()
                        } label: {
                            Text(theme.messagingFormat(.voiceSpeedFormat, Double(voicePlayer.playbackRate)))
                                .fabushiFont(.caption2)
                                .fontWeight(.semibold)
                                .foregroundStyle(bubbleTextColor)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.black.opacity(0.14), in: Capsule())
                        }
                        .buttonStyle(.plain)

                        Button {
                            Task { @MainActor in
                                await voicePlayer.transcribeVoiceNote(
                                    fileId: info.fileId,
                                    chatId: chatId,
                                    store: store,
                                    router: router,
                                    isCompact: horizontalSizeClass == .compact,
                                    locale: theme.locale
                                )
                            }
                        } label: {
                            Text(theme.messagingString(.voiceTranscribeAction))
                                .fabushiFont(.caption2)
                                .foregroundStyle(bubbleTextColor)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.black.opacity(0.14), in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            if let transcript, !transcript.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text(theme.messagingString(.voiceTranscriptTitle))
                        .fabushiFont(.caption2)
                        .foregroundStyle(bubbleSecondaryTextColor)
                    Text(transcript)
                        .fabushiFont(.callout)
                        .foregroundStyle(bubbleTextColor)
                }
                .padding(8)
                .background(Color.black.opacity(0.12), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
        }
        .frame(minWidth: 210)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.mediaPlayerCard)
    }

    // MARK: - 4. File / Document Card (`D10-08` Authoritative Download Failure Toast)

    private var fileDocumentCardView: some View {
        let info: (fileId: String, fileName: String, mimeType: String, sizeBytes: UInt64) = {
            switch message.content {
            case .document(let fid, let name, let mime, let size):
                return (fid, name, mime, size)
            case .media(let media):
                return (media.fileId, media.fileName ?? media.fileId, media.mimeType, media.sizeBytes)
            default:
                return ("doc-\(message.messageId)", "attachment.bin", "application/octet-stream", 0)
            }
        }()

        return Button {
            guard !isDownloadingFile else { return }
            isDownloadingFile = true
            Task { @MainActor in
                let data = await store.downloadAttachment(
                    fileId: info.fileId,
                    fileName: info.fileName,
                    chatId: chatId,
                    router: router,
                    isCompact: horizontalSizeClass == .compact,
                    locale: theme.locale
                )
                isDownloadingFile = false
                if data != nil {
                    store.overlayController.openMediaLightbox(
                        FabushiMediaViewerTarget(
                            fileId: info.fileId,
                            fileName: info.fileName,
                            mimeType: info.mimeType,
                            caption: nil,
                            isVideo: false,
                            chatId: chatId
                        )
                    )
                }
            }
        } label: {
            HStack(spacing: 10) {
                ZStack {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(Color.black.opacity(0.14))
                        .frame(width: 42, height: 42)
                    if isDownloadingFile {
                        ProgressView().controlSize(.small)
                    } else {
                        Image(systemName: "doc.fill")
                            .fabushiFont(.headline)
                            .foregroundStyle(bubbleTextColor)
                    }
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(info.fileName)
                        .fabushiFont(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(bubbleTextColor)
                        .lineLimit(1)
                    Text("\(ByteCountFormatter.string(fromByteCount: Int64(info.sizeBytes), countStyle: .file)) • \(info.mimeType)")
                        .fabushiFont(.caption2)
                        .foregroundStyle(bubbleSecondaryTextColor)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                Image(systemName: "arrow.down.circle")
                    .fabushiFont(.body)
                    .foregroundStyle(bubbleTextColor)
            }
            .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
    }

    // MARK: - 5. Sticker, 6. Album, 7. Location

    private var stickerCardView: some View {
        let emoji = parsedExtendedField("emoji") ?? (message.text.isEmpty ? "✨" : message.text)
        return VStack(alignment: message.isOutgoing ? .trailing : .leading, spacing: 4) {
            Text(emoji)
                .fabushiFont(.largeTitle)
                .scaleEffect(1.8)
                .frame(width: 92, height: 92)
            Text(formattedTime(message.dateMs))
                .fabushiFont(.caption2)
                .foregroundStyle(theme.textMuted)
        }
        .padding(4)
    }

    private var albumCardView: some View {
        let items = parsedExtendedArray("items")
        let displayCount = max(2, min(4, items.count))
        return VStack(alignment: .leading, spacing: 6) {
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 4), GridItem(.flexible(), spacing: 4)], spacing: 4) {
                ForEach(0..<displayCount, id: \.self) { idx in
                    let fileId = idx < items.count ? (items[idx]["file_id"]?.stringValue ?? "album-\(message.messageId)-\(idx)") : "album-\(idx)"
                    Button {
                        store.overlayController.openMediaLightbox(
                            FabushiMediaViewerTarget(
                                fileId: fileId,
                                fileName: "album-\(idx + 1).jpg",
                                mimeType: "image/jpeg",
                                caption: message.text,
                                isVideo: false,
                                chatId: chatId
                            )
                        )
                    } label: {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .fill(theme.surfaceElevated)
                            .frame(height: 82)
                            .overlay(
                                Image(systemName: "photo")
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.accentColor)
                            )
                    }
                    .buttonStyle(.plain)
                }
            }
            .frame(width: 210)
            if !message.text.isEmpty {
                Text(message.text)
                    .fabushiFont(.body)
                    .foregroundStyle(bubbleTextColor)
            }
        }
    }

    private var locationCardView: some View {
        let lat = parsedExtendedDouble("latitude") ?? 0
        let lon = parsedExtendedDouble("longitude") ?? 0
        let coordText = String(format: "%.5f, %.5f", lat, lon)
        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "mappin.and.ellipse")
                    .fabushiFont(.title3)
                    .foregroundStyle(bubbleTextColor)
                VStack(alignment: .leading, spacing: 2) {
                    Text(theme.messagingString(.locationCardTitle))
                        .fabushiFont(.subheadline)
                        .fontWeight(.semibold)
                        .foregroundStyle(bubbleTextColor)
                    Text(coordText)
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(bubbleSecondaryTextColor)
                }
            }
            if let mapsURL = URL(string: "https://maps.apple.com/?ll=\(lat),\(lon)") {
                Link(destination: mapsURL) {
                    Label(theme.messagingString(.openInMapsAction), systemImage: "map")
                        .fabushiFont(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(bubbleTextColor)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(Color.black.opacity(0.14), in: Capsule())
                }
            }
        }
    }

    // MARK: - 8. Poll & 9. Todo Checklist

    private func pollCardView(_ poll: FabushiPollCard) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(poll.question)
                .fabushiFont(.headline)
                .foregroundStyle(bubbleTextColor)

            ForEach(poll.options) { option in
                Button {
                    guard !poll.isClosed else { return }
                    Task { @MainActor in
                        _ = await store.votePoll(
                            chatId: chatId,
                            messageId: message.messageId,
                            pollId: poll.pollId,
                            optionIds: [option.optionId]
                        )
                    }
                } label: {
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            Image(systemName: option.isChosen ? "checkmark.circle.fill" : "circle")
                                .fabushiFont(.caption)
                            Text(option.text)
                                .fabushiFont(.subheadline)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if poll.totalVoters > 0 || poll.isClosed {
                                Text("\(Int(round(option.percentage)))%")
                                    .fabushiFont(.caption2)
                                    .fontWeight(.semibold)
                            }
                        }
                        .foregroundStyle(bubbleTextColor)

                        GeometryReader { proxy in
                            ZStack(alignment: .leading) {
                                Capsule().fill(Color.black.opacity(0.14))
                                Capsule()
                                    .fill(message.isOutgoing ? theme.bubbleOutgoingText.opacity(0.8) : theme.accentColor)
                                    .frame(width: proxy.size.width * CGFloat(min(100, max(0, option.percentage)) / 100.0))
                            }
                        }
                        .frame(height: 5)
                    }
                    .padding(.vertical, 4)
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(poll.isClosed)
            }

            if let solution = poll.solution, !solution.isEmpty {
                Text(solution)
                    .fabushiFont(.caption)
                    .foregroundStyle(bubbleSecondaryTextColor)
            }

            HStack {
                Text(
                    poll.isClosed
                        ? theme.messagingString(.pollClosedBadge)
                        : theme.messagingFormat(.pollVotesCountFormat, poll.totalVoters)
                )
                .fabushiFont(.caption2)
                .foregroundStyle(bubbleSecondaryTextColor)
                Spacer()
                if !poll.isClosed && message.isOutgoing {
                    Button {
                        Task { @MainActor in
                            _ = await store.closePoll(
                                chatId: chatId,
                                messageId: message.messageId,
                                pollId: poll.pollId
                            )
                        }
                    } label: {
                        Text(theme.messagingString(.pollCloseAction))
                            .fabushiFont(.caption2)
                            .fontWeight(.semibold)
                            .foregroundStyle(bubbleTextColor)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .frame(minWidth: 210)
    }

    private func todoCardView(_ todo: FabushiTodoCard) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(todo.title)
                    .fabushiFont(.headline)
                    .foregroundStyle(bubbleTextColor)
                Spacer()
                Text("\(todo.completedCount)/\(todo.items.count)")
                    .fabushiFont(.caption2)
                    .foregroundStyle(bubbleSecondaryTextColor)
            }

            ForEach(todo.items) { task in
                Button {
                    store.toggleTodoItem(chatId: chatId, messageId: message.messageId, itemId: task.id)
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: task.completed ? "checkmark.square.fill" : "square")
                            .fabushiFont(.subheadline)
                            .foregroundStyle(bubbleTextColor)
                        Text(task.title)
                            .fabushiFont(.callout)
                            .strikethrough(task.completed)
                            .foregroundStyle(task.completed ? bubbleSecondaryTextColor : bubbleTextColor)
                        Spacer()
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
            }
        }
        .frame(minWidth: 200)
    }

    // MARK: - 10. Service, 11. Call, 12. Story Mention, 13. Invoice

    private var servicePillView: some View {
        Text(message.text)
            .fabushiFont(.caption)
            .foregroundStyle(Color(fabushiHex: theme.palette.servicePillForegroundRGB))
            .padding(.horizontal, 12)
            .padding(.vertical, 5)
            .background(
                Color(fabushiHex: theme.palette.servicePillBackgroundRGB),
                in: Capsule()
            )
            .padding(.vertical, 4)
    }

    private var callBubbleView: some View {
        let isMissed = (parsedExtendedField("status") ?? "").lowercased() == "missed"
        let durationSec = UInt32(parsedExtendedDouble("duration_seconds") ?? 0)
        return HStack(spacing: 10) {
            Image(systemName: isMissed ? "phone.down.fill" : "phone.fill")
                .fabushiFont(.headline)
                .foregroundStyle(isMissed ? theme.destructiveColor : bubbleTextColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(
                    isMissed
                        ? theme.messagingString(.callMissedLabel)
                        : (message.isOutgoing
                            ? theme.messagingString(.callOutgoingLabel)
                            : theme.messagingString(.callIncomingLabel))
                )
                .fabushiFont(.subheadline)
                .fontWeight(.semibold)
                .foregroundStyle(bubbleTextColor)
                if durationSec > 0 {
                    Text(formatDuration(durationSec))
                        .fabushiFont(.caption2)
                        .foregroundStyle(bubbleSecondaryTextColor)
                }
            }
            Spacer(minLength: 8)
            Button {
                router.navigate(to: .calls, coordinator: coordinator)
            } label: {
                Text(theme.messagingString(.callBackAction))
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(bubbleTextColor)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Color.black.opacity(0.14), in: Capsule())
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
        }
    }

    private var storyMentionCardView: some View {
        Button {
            store.overlayController.openStories(startingAt: 0)
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "circle.dashed.inset.filled")
                    .fabushiFont(.title3)
                    .foregroundStyle(bubbleTextColor)
                VStack(alignment: .leading, spacing: 2) {
                    Text(theme.messagingString(.storyReplyBadge))
                        .fabushiFont(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(bubbleSecondaryTextColor)
                    Text(message.text)
                        .fabushiFont(.callout)
                        .foregroundStyle(bubbleTextColor)
                        .lineLimit(2)
                }
            }
            .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
    }

    private func invoiceCardView(_ invoice: FabushiInvoiceCard) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(invoice.title)
                    .fabushiFont(.headline)
                    .foregroundStyle(bubbleTextColor)
                Spacer()
                Text(
                    invoice.isPaid
                        ? theme.messagingString(.invoicePaidBadge)
                        : theme.messagingString(.invoicePendingBadge)
                )
                .fabushiFont(.caption2)
                .fontWeight(.semibold)
                .foregroundStyle(invoice.isPaid ? theme.successColor : theme.warningColor)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(Color.black.opacity(0.14), in: Capsule())
            }
            if !invoice.descriptionText.isEmpty {
                Text(invoice.descriptionText)
                    .fabushiFont(.callout)
                    .foregroundStyle(bubbleSecondaryTextColor)
            }
            Text(String(format: "%@ %.2f", invoice.currency.uppercased(), Double(invoice.amountCents) / 100.0))
                .fabushiFont(.title3)
                .foregroundStyle(bubbleTextColor)
        }
        .frame(minWidth: 200)
    }

    // MARK: - 14. Generative, Coding Diff, Browser Takeover (`D11-02`), Distilled Script, Computer & Encrypted

    private func generativeCardView(_ card: FabushiGenerativeCard) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(card.title)
                .fabushiFont(.headline)
                .foregroundStyle(bubbleTextColor)
            if let subtitle = card.subtitle, !subtitle.isEmpty {
                Text(subtitle)
                    .fabushiFont(.callout)
                    .foregroundStyle(bubbleSecondaryTextColor)
            }
            if let status = card.status, !status.isEmpty {
                Text(status)
                    .fabushiFont(.caption)
                    .foregroundStyle(bubbleTextColor)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(Color.black.opacity(0.14), in: Capsule())
            }
        }
    }

    private var codingDiffCardView: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(message.text)
                .fabushiFont(.monospacedCaption)
                .foregroundStyle(bubbleTextColor)
            Button {
                router.navigate(to: .agentStudio(tab: "coding"), coordinator: coordinator)
            } label: {
                Label("Review Diff", systemImage: "doc.text.magnifyingglass")
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(bubbleTextColor)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Color.black.opacity(0.16), in: Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.codingCheckpointButton)
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.codingDiffCard)
    }

    private var browserTakeoverCardView: some View {
        let sessionId = parsedExtendedField("session_id") ?? "active"
        return VStack(alignment: .leading, spacing: 8) {
            Label(theme.messagingString(.browserTakeoverAction), systemImage: "safari.fill")
                .fabushiFont(.headline)
                .foregroundStyle(bubbleTextColor)
            if !message.text.isEmpty {
                Text(message.text)
                    .fabushiFont(.callout)
                    .foregroundStyle(bubbleSecondaryTextColor)
            }
            if message.inlineKeyboard.isEmpty {
                HStack(spacing: 8) {
                    inlineBrowserActionButton(
                        title: theme.messagingString(.browserTakeoverAction),
                        callback: "browser_takeover:\(sessionId)"
                    )
                    inlineBrowserActionButton(
                        title: theme.messagingString(.browserResumeAction),
                        callback: "browser_resume:\(sessionId)"
                    )
                    inlineBrowserActionButton(
                        title: theme.messagingString(.browserCancelAction),
                        callback: "browser_cancel:\(sessionId)"
                    )
                }
            }
        }
    }

    private func inlineBrowserActionButton(title: String, callback: String) -> some View {
        Button {
            Task { @MainActor in
                await store.handleInlineKeyboardAction(
                    button: FabushiInlineKeyboardButton(text: title, callbackData: callback),
                    chatId: chatId,
                    messageId: message.messageId,
                    router: router,
                    isCompact: horizontalSizeClass == .compact,
                    locale: theme.locale
                )
            }
        } label: {
            Text(title)
                .fabushiFont(.caption2)
                .fontWeight(.semibold)
                .foregroundStyle(bubbleTextColor)
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(Color.black.opacity(0.16), in: Capsule())
        }
        .buttonStyle(.plain)
    }

    private var distilledScriptInlineCardView: some View {
        let name = parsedExtendedField("name") ?? message.text
        return VStack(alignment: .leading, spacing: 6) {
            Label(theme.messagingString(.slashDistillCardTitle), systemImage: "wand.and.stars")
                .fabushiFont(.headline)
                .foregroundStyle(bubbleTextColor)
            Text(name)
                .fabushiFont(.callout)
                .foregroundStyle(bubbleSecondaryTextColor)
        }
    }

    private var computerCardView: some View {
        let nodeId = parsedExtendedField("node_id")
        return VStack(alignment: .leading, spacing: 8) {
            Label(message.text.isEmpty ? theme.messagingString(.inspectorBotComputersTitle) : message.text, systemImage: "desktopcomputer")
                .fabushiFont(.subheadline)
                .fontWeight(.semibold)
                .foregroundStyle(bubbleTextColor)
            Button {
                router.navigate(to: .remoteComputer(nodeId: nodeId), coordinator: coordinator)
            } label: {
                Text(theme.messagingString(.computerCardOpenAction))
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(bubbleTextColor)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Color.black.opacity(0.16), in: Capsule())
            }
            .buttonStyle(.plain)
        }
    }

    private var encryptedMessageCardView: some View {
        HStack(spacing: 8) {
            Image(systemName: "lock.fill")
                .fabushiFont(.subheadline)
                .foregroundStyle(bubbleTextColor)
            Text(theme.messagingString(.e2eeEncryptedMessageLabel))
                .fabushiFont(.callout)
                .foregroundStyle(bubbleTextColor)
        }
    }

    // MARK: - Inline Keyboard, Reactions & Reply Header

    private var inlineKeyboardView: some View {
        VStack(spacing: 4) {
            ForEach(Array(message.inlineKeyboard.enumerated()), id: \.offset) { _, row in
                HStack(spacing: 4) {
                    ForEach(row) { button in
                        Button {
                            Task { @MainActor in
                                await store.handleInlineKeyboardAction(
                                    button: button,
                                    chatId: chatId,
                                    messageId: message.messageId,
                                    router: router,
                                    isCompact: horizontalSizeClass == .compact,
                                    locale: theme.locale
                                )
                            }
                        } label: {
                            Text(button.text)
                                .fabushiFont(.caption)
                                .fontWeight(.semibold)
                                .foregroundStyle(theme.accentColor)
                                .frame(maxWidth: .infinity)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
                                .background(
                                    theme.surfaceElevated,
                                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                                )
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .frame(maxWidth: 280)
    }

    private var reactionsRowView: some View {
        HStack(spacing: 6) {
            ForEach(message.reactions) { reaction in
                Button {
                    onReactionTap(reaction.emoji)
                } label: {
                    HStack(spacing: 4) {
                        Text(reaction.emoji)
                            .fabushiFont(.caption)
                        Text("\(reaction.count)")
                            .fabushiFont(.caption2)
                            .fontWeight(.semibold)
                            .foregroundStyle(reaction.isChosenByMe ? theme.accentForegroundColor : theme.textSecondary)
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(
                        reaction.isChosenByMe ? theme.accentColor : theme.surfaceElevated,
                        in: Capsule()
                    )
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.reactMessageAction)
            }
        }
    }

    private func replyHeaderStrip(_ reply: FabushiReplyHeader) -> some View {
        Button(action: onReplyTap) {
            HStack(spacing: 8) {
                Capsule()
                    .fill(message.isOutgoing ? theme.bubbleOutgoingText : theme.accentColor)
                    .frame(width: 3)
                VStack(alignment: .leading, spacing: 1) {
                    if let name = reply.senderName, !name.isEmpty {
                        Text(name)
                            .fabushiFont(.caption2)
                            .fontWeight(.semibold)
                            .foregroundStyle(bubbleTextColor)
                            .lineLimit(1)
                    }
                    Text(reply.quoteText ?? "#\(reply.replyToMsgId)")
                        .fabushiFont(.caption)
                        .foregroundStyle(bubbleSecondaryTextColor)
                        .lineLimit(2)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color.black.opacity(0.10), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var deliveryTickIcon: some View {
        switch message.deliveryStatus {
        case .pending, .sending:
            Image(systemName: "clock")
                .fabushiFont(.caption2)
                .foregroundStyle(bubbleSecondaryTextColor)
        case .sent:
            Image(systemName: "checkmark")
                .fabushiFont(.caption2)
                .foregroundStyle(bubbleSecondaryTextColor)
        case .read:
            Image(systemName: "checkmark.circle.fill")
                .fabushiFont(.caption2)
                .foregroundStyle(bubbleTextColor)
        case .failed:
            Image(systemName: "exclamationmark.circle.fill")
                .fabushiFont(.caption2)
                .foregroundStyle(theme.destructiveColor)
        }
    }

    // MARK: - Formatting, Parsing & Geometry Helpers

    private var senderLabel: String {
        if let contact = store.contacts.first(where: { $0.userId == message.senderId }) {
            return contact.displayName
        }
        if let bot = store.bots.first(where: { $0.botId == message.senderId }) {
            return bot.displayName
        }
        return "User \(message.senderId)"
    }

    private func bubbleRadii() -> RectangleCornerRadii {
        let r = theme.bubbleCornerRadius
        let small: CGFloat = item.showsTail ? 4 : max(6, r * 0.45)
        if message.isOutgoing {
            return RectangleCornerRadii(
                topLeading: r,
                bottomLeading: r,
                bottomTrailing: small,
                topTrailing: (item.position == .middle || item.position == .last) ? small : r
            )
        } else {
            return RectangleCornerRadii(
                topLeading: (item.position == .middle || item.position == .last) ? small : r,
                bottomLeading: small,
                bottomTrailing: r,
                topTrailing: r
            )
        }
    }

    private func parsedExtendedJSON() -> FabushiJSONValue? {
        guard case .extended(_, let rawJSON)? = message.content,
              let data = rawJSON.data(using: .utf8)
        else { return nil }
        return try? JSONDecoder().decode(FabushiJSONValue.self, from: data)
    }

    private func parsedExtendedField(_ key: String) -> String? {
        parsedExtendedJSON()?[key]?.stringValue
    }

    private func parsedExtendedDouble(_ key: String) -> Double? {
        if let d = parsedExtendedJSON()?[key]?.doubleValue { return d }
        if let i = parsedExtendedJSON()?[key]?.int64Value { return Double(i) }
        return nil
    }

    private func parsedExtendedArray(_ key: String) -> [FabushiJSONValue] {
        parsedExtendedJSON()?[key]?.arrayValue ?? []
    }

    private func extractFirstHTTPURL(from text: String, entities: [FabushiMessageEntity]) -> String? {
        for entity in entities {
            if case .textUrl(let url) = entity.entityType,
               url.hasPrefix("http://") || url.hasPrefix("https://") {
                return url
            }
        }
        for token in text.split(whereSeparator: \.isWhitespace) {
            let str = String(token)
            if str.hasPrefix("https://") || str.hasPrefix("http://") {
                return str
            }
        }
        return nil
    }

    private func formattedTime(_ ms: Int64) -> String {
        guard ms > 0 else { return "" }
        let date = Date(timeIntervalSince1970: Double(ms) / 1000.0)
        return date.formatted(date: .omitted, time: .shortened)
    }

    private func formatDuration(_ seconds: UInt32) -> String {
        String(format: "%d:%02d", seconds / 60, seconds % 60)
    }

    private struct TextSegment {
        let isCode: Bool
        let language: String?
        let content: String
    }

    private static func splitFencedCodeBlocks(_ text: String) -> [TextSegment] {
        guard text.contains("```") else {
            return [TextSegment(isCode: false, language: nil, content: text)]
        }
        var output: [TextSegment] = []
        let lines = text.components(separatedBy: "\n")
        var inCode = false
        var currentLang: String? = nil
        var buffer: [String] = []

        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("```") {
                if inCode {
                    output.append(TextSegment(isCode: true, language: currentLang, content: buffer.joined(separator: "\n")))
                    buffer.removeAll()
                    currentLang = nil
                    inCode = false
                } else {
                    if !buffer.isEmpty {
                        output.append(TextSegment(isCode: false, language: nil, content: buffer.joined(separator: "\n")))
                        buffer.removeAll()
                    }
                    let lang = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                    currentLang = lang.isEmpty ? nil : lang
                    inCode = true
                }
            } else {
                buffer.append(line)
            }
        }
        if !buffer.isEmpty {
            output.append(TextSegment(isCode: inCode, language: currentLang, content: buffer.joined(separator: "\n")))
        }
        return output.isEmpty ? [TextSegment(isCode: false, language: nil, content: text)] : output
    }

    public static func attributedFormattedText(
        _ text: String,
        entities: [FabushiMessageEntity]
    ) -> AttributedString {
        if let parsedMarkdown = try? AttributedString(
            markdown: text,
            options: AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        ) {
            return parsedMarkdown
        }
        return AttributedString(text)
    }
}

// MARK: - 15. Agent Thinking Bubble (`D14-04`: Works in BOTH 1:1 Bot DMs & Group Chats)

@MainActor
public struct FabushiAgentThinkingBubbleView: View {
    public let activity: FabushiBotActivityState

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
    @State private var isExpanded: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var reduceMotion: Bool {
        theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)
    }

    public init(activity: FabushiBotActivityState) {
        self.activity = activity
    }

    public var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 6) {
                Button {
                    isExpanded.toggle()
                } label: {
                    HStack(spacing: 8) {
                        if reduceMotion {
                            Image(systemName: "sparkles")
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.accentColor)
                        } else {
                            ProgressView().controlSize(.mini)
                        }
                        Text(theme.messagingString(.botThinkingIndicator))
                            .fabushiFont(.subheadline)
                            .fontWeight(.semibold)
                            .foregroundStyle(theme.textPrimary)
                        Spacer(minLength: 8)
                        if activity.detail != nil {
                            Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)

                if isExpanded, let detail = activity.detail, !detail.isEmpty {
                    Text(detail)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(
                theme.bubbleIncoming,
                in: RoundedRectangle(cornerRadius: theme.bubbleCornerRadius, style: .continuous)
            )
            Spacer(minLength: 48)
        }
        .padding(.horizontal, 12)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.streamingIndicator)
    }
}

// MARK: - 16. Tool Permission Dock (`fabushi.approvalsList` / `fabushi.approvalDecide`)

@MainActor
public struct FabushiToolPermissionDockView: View {
    public let approval: FabushiApprovalItem
    public let onApprove: () -> Void
    public let onDeny: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        approval: FabushiApprovalItem,
        onApprove: @escaping () -> Void,
        onDeny: @escaping () -> Void
    ) {
        self.approval = approval
        self.onApprove = onApprove
        self.onDeny = onDeny
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "shield.lefthalf.filled")
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.warningColor)
                Text(approval.patternName ?? theme.messagingString(.agentReasoningTitle))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                Spacer()
                Text(approval.riskLevel.uppercased())
                    .fabushiFont(.caption2)
                    .fontWeight(.bold)
                    .foregroundStyle(theme.warningColor)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .background(theme.warningColor.opacity(0.14), in: Capsule())
            }

            Text(approval.command)
                .fabushiFont(.monospacedCaption)
                .foregroundStyle(theme.textSecondary)
                .lineLimit(3)

            HStack(spacing: 10) {
                FabushiGlassPillButton(
                    title: theme.messagingString(.toolApproveAction),
                    systemImage: "checkmark",
                    accessibilityID: FabushiAccessibilityIDs.Stage.toolApproveButton,
                    isProminent: true,
                    action: onApprove
                )
                FabushiGlassPillButton(
                    title: theme.messagingString(.toolDenyAction),
                    systemImage: "xmark",
                    accessibilityID: FabushiAccessibilityIDs.Stage.toolDenyButton,
                    isProminent: false,
                    action: onDeny
                )
            }
        }
        .padding(14)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius)
        .padding(.horizontal, 12)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.toolApprovalDock)
    }
}

// MARK: - Distilled Browser Script Summary Card (`D11-01` / `D11-02`)

@MainActor
public struct FabushiDistilledScriptCardView: View {
    public let script: FabushiDistilledBrowserScript
    public let onOpenStudio: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(script: FabushiDistilledBrowserScript, onOpenStudio: @escaping () -> Void) {
        self.script = script
        self.onOpenStudio = onOpenStudio
    }

    public var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "wand.and.stars")
                .fabushiFont(.headline)
                .foregroundStyle(theme.accentColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(theme.messagingString(.slashDistillCardTitle))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                Text(script.name)
                    .fabushiFont(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(1)
            }
            Spacer()
            Button(action: onOpenStudio) {
                Text(theme.messagingString(.prestartBotOpenMiniAppAction))
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.accentForegroundColor)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.accentColor, in: Capsule())
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
        }
        .padding(12)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .padding(.horizontal, 12)
    }
}
