import CoreLocation
import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Formatting Helper (UTF-16 `FabushiMessageEntity` Builder)

public enum FabushiComposerFormattingKind: String, CaseIterable, Sendable, Identifiable {
    case bold
    case italic
    case code
    case strikethrough
    case spoiler
    case link

    public var id: String { rawValue }

    public var stringKey: FabushiMessagingStrings.Key {
        switch self {
        case .bold: return .formatBoldAction
        case .italic: return .formatItalicAction
        case .code: return .formatCodeAction
        case .strikethrough: return .formatStrikethroughAction
        case .spoiler: return .formatSpoilerAction
        case .link: return .formatLinkAction
        }
    }

    public var systemImage: String {
        switch self {
        case .bold: return "bold"
        case .italic: return "italic"
        case .code: return "chevron.left.forwardslash.chevron.right"
        case .strikethrough: return "strikethrough"
        case .spoiler: return "eye.slash"
        case .link: return "link"
        }
    }
}

public enum FabushiComposerFormattingHelper: Sendable {
    public static func apply(
        _ kind: FabushiComposerFormattingKind,
        to currentText: String
    ) -> (text: String, entity: FabushiMessageEntity?) {
        let trimmed = currentText.trimmingCharacters(in: .whitespacesAndNewlines)
        let prefix: String
        let suffix: String
        let placeholder: String
        let entityKind: FabushiMessageEntityKind

        switch kind {
        case .bold:
            prefix = "**"; suffix = "**"; placeholder = "bold"; entityKind = .bold
        case .italic:
            prefix = "_"; suffix = "_"; placeholder = "italic"; entityKind = .italic
        case .code:
            prefix = "`"; suffix = "`"; placeholder = "code"; entityKind = .code
        case .strikethrough:
            prefix = "~~"; suffix = "~~"; placeholder = "strike"; entityKind = .strikethrough
        case .spoiler:
            prefix = "||"; suffix = "||"; placeholder = "spoiler"; entityKind = .spoiler
        case .link:
            prefix = "["; suffix = "](https://)"; placeholder = "link"; entityKind = .textUrl(url: "https://")
        }

        if trimmed.isEmpty {
            let wrapped = "\(prefix)\(placeholder)\(suffix)"
            let entity = FabushiMessageEntity(
                offset: 0,
                length: UInt32(wrapped.utf16.count),
                entityType: entityKind
            )
            return (wrapped, entity)
        }

        let offset = UInt32(currentText.utf16.count)
        let addition = (currentText.hasSuffix(" ") || currentText.hasSuffix("\n") ? "" : " ") + "\(prefix)\(placeholder)\(suffix)"
        let entity = FabushiMessageEntity(
            offset: offset,
            length: UInt32(addition.utf16.count),
            entityType: entityKind
        )
        return (currentText + addition, entity)
    }
}

// MARK: - Native Document & Camera Pickers

#if canImport(UIKit)
@MainActor
private struct FabushiDocumentPickerSheet: UIViewControllerRepresentable {
    let onPicked: (URL) -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onPicked: onPicked, onCancel: onCancel)
    }

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let controller = UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: true)
        controller.delegate = context.coordinator
        controller.allowsMultipleSelection = false
        return controller
    }

    func updateUIViewController(_ uiViewController: UIDocumentPickerViewController, context: Context) {}

    @MainActor
    final class Coordinator: NSObject, UIDocumentPickerDelegate {
        let onPicked: (URL) -> Void
        let onCancel: () -> Void

        init(onPicked: @escaping (URL) -> Void, onCancel: @escaping () -> Void) {
            self.onPicked = onPicked
            self.onCancel = onCancel
        }

        func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
            if let first = urls.first {
                onPicked(first)
            } else {
                onCancel()
            }
        }

        func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
            onCancel()
        }
    }
}

@MainActor
private struct FabushiCameraPickerSheet: UIViewControllerRepresentable {
    let onCaptured: (UIImage) -> Void
    let onCancel: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onCaptured: onCaptured, onCancel: onCancel)
    }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ uiViewController: UIImagePickerController, context: Context) {}

    @MainActor
    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let onCaptured: (UIImage) -> Void
        let onCancel: () -> Void

        init(onCaptured: @escaping (UIImage) -> Void, onCancel: @escaping () -> Void) {
            self.onCaptured = onCaptured
            self.onCancel = onCancel
        }

        func imagePickerController(
            _ picker: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            if let image = info[.originalImage] as? UIImage {
                onCaptured(image)
            } else {
                onCancel()
            }
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            onCancel()
        }
    }
}
#endif

// MARK: - Canonical IME-Safe Apple HIG Composer Bar (`#15`, `#16`, `D10-07`, `D11-01`, `D14-03`)

@MainActor
public struct FabushiComposerBarView: View {
    public let chatId: Int64
    public let chat: FabushiMessagingChat?
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var composerText: String = ""
    @State private var isComposingMarkedText: Bool = false
    @State private var measuredInputHeight: CGFloat = 38
    @State private var pendingEntities: [FabushiMessageEntity] = []
    @State private var contextBadges: [FabushiContextBadge] = []
    @State private var showFormattingToolbar: Bool = false
    @State private var showSlashPopoverExplicitly: Bool = false
    @State private var showReplyKeyboard: Bool = false
    @State private var validationErrorText: String? = nil

    // Attachment & Media Sheets
    @State private var isPhotoPickerPresented: Bool = false
    @State private var selectedPhotoPickerItem: PhotosPickerItem? = nil
    #if canImport(UIKit)
    @State private var pendingPhotoForEditor: UIImage? = nil
    #endif
    @State private var isDocumentPickerPresented: Bool = false
    @State private var isCameraPresented: Bool = false
    @State private var isLocationSheetPresented: Bool = false
    @State private var isPollCreatorPresented: Bool = false
    @State private var isTodoCreatorPresented: Bool = false
    @State private var isStickerPickerPresented: Bool = false
    @State private var isContextBadgePickerPresented: Bool = false
    @State private var isSendAsSheetPresented: Bool = false
    @State private var isScheduleSheetPresented: Bool = false
    @State private var scheduledDate: Date = Date().addingTimeInterval(1800)

    // Real AVFoundation Voice Recording & Hold/Slide-to-Cancel/Lock Gesture
    @State private var voiceRecorder = VoiceRecorder()
    @State private var locationService = LocationService()
    @State private var isVoiceRecordingLocked: Bool = false
    @State private var voiceDragOffset: CGSize = .zero
    @State private var didCancelVoiceGesture: Bool = false
    @State private var draftSaveTask: Task<Void, Never>? = nil
    @State private var typingTask: Task<Void, Never>? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var editingMessage: FabushiMessageRecord? {
        store.editingMessageByChat[chatId]
    }

    private var replyTarget: FabushiReplyTarget? {
        store.replyTargetByChat[chatId]
    }

    private var activeSlashPrefix: String? {
        if showSlashPopoverExplicitly && !isComposingMarkedText {
            return composerText.hasPrefix("/") ? composerText : "/"
        }
        return FabushiIMECompositionGuard.activeSlashPrefix(
            text: composerText,
            hasMarkedText: isComposingMarkedText
        )
    }

    private var canSendText: Bool {
        !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !contextBadges.isEmpty
    }

    private var sendAsPeers: [FabushiSendAsPeer] {
        store.sendAsPeersByChat[chatId] ?? []
    }

    private var botReplyRows: [[String]] {
        store.botReplyKeyboardByChat[chatId] ?? []
    }

    public init(
        chatId: Int64,
        chat: FabushiMessagingChat?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore
    ) {
        self.chatId = chatId
        self.chat = chat
        self.coordinator = coordinator
        self.router = router
        self.store = store
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Slash Command Autocomplete Popover (`#16`, `D11-01`)
            if let slashPrefix = activeSlashPrefix, !store.slashCommands.isEmpty {
                FabushiSlashCommandPopoverView(
                    commands: store.slashCommands,
                    prefix: slashPrefix,
                    onSelectCommand: { command in
                        guard FabushiIMECompositionGuard.shouldAllowCommandOrMentionCommit(
                            hasMarkedText: isComposingMarkedText
                        ) else { return }
                        showSlashPopoverExplicitly = false
                        if command.acceptsArgs {
                            composerText = "\(command.slashName) "
                        } else {
                            composerText = command.slashName
                            submitPrimaryMessage(silent: false, scheduledAtMs: nil)
                        }
                    },
                    onDismiss: {
                        showSlashPopoverExplicitly = false
                    }
                )
            }

            if let validationErrorText {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.destructiveColor)
                    Text(validationErrorText)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.destructiveColor)
                    Spacer()
                    Button {
                        self.validationErrorText = nil
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textMuted)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 6)
                .background(theme.destructiveColor.opacity(0.10))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.validationErrorBanner)
            }

            // Reply / Edit Banner (`D14-03`)
            if let editing = editingMessage {
                replyOrEditBanner(
                    icon: "pencil",
                    title: theme.messagingString(.composerEditingLabel),
                    snippet: editing.text,
                    onDismiss: {
                        store.editingMessageByChat.removeValue(forKey: chatId)
                        composerText = store.unsentDraftTextByChat[chatId] ?? ""
                    }
                )
            } else if let reply = replyTarget {
                replyOrEditBanner(
                    icon: "arrowshape.turn.up.left.fill",
                    title: theme.messagingFormat(.composerReplyPrefixFormat, reply.senderName),
                    snippet: reply.quoteText,
                    onDismiss: {
                        store.replyTargetByChat.removeValue(forKey: chatId)
                    }
                )
            }

            // Context Badge Strip (`@file`, `@dir`, `@diff`, `@git:`, `@url`)
            if !contextBadges.isEmpty {
                FabushiContextBadgeStripView(
                    badges: contextBadges,
                    onRemove: { id in
                        contextBadges.removeAll { $0.id == id }
                    },
                    onAddTapped: {
                        isContextBadgePickerPresented = true
                    }
                )
            }

            // Formatting Toolbar
            if showFormattingToolbar {
                formattingToolbarStrip
            }

            Divider().overlay(theme.separatorHairline)

            // Main Composer Row (or Active Voice Recording HUD)
            if voiceRecorder.isRecording {
                voiceRecordingActiveRow
            } else {
                mainInputRow
            }

            // Bot Reply Keyboard (`D10-07`: non-composer sends preserve unsent draft!)
            if showReplyKeyboard && !botReplyRows.isEmpty {
                botReplyKeyboardGrid
            }
        }
        .background(theme.surfaceComposer)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.root)
        .task(id: chatId) {
            syncComposerFromStoreDraft()
            if store.slashCommands.isEmpty {
                await store.refreshSlashCommands()
            }
        }
        .onChange(of: store.unsentDraftTextByChat[chatId]) { _, restoredDraft in
            // `D10-07`: Reflect failed-send draft restoration or remote draft sync when not mid-IME!
            guard !isComposingMarkedText, editingMessage == nil else { return }
            let nextText = restoredDraft ?? ""
            if composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !nextText.isEmpty {
                composerText = nextText
            }
        }
        .onChange(of: store.editingMessageByChat[chatId]?.messageId) { _, _ in
            if let editing = store.editingMessageByChat[chatId] {
                composerText = editing.text
            }
        }
        .onChange(of: composerText) { _, newValue in
            guard editingMessage == nil else { return }
            scheduleDebouncedDraftSave(text: newValue)
            scheduleTypingPing(text: newValue)
        }
        .photosPicker(
            isPresented: $isPhotoPickerPresented,
            selection: $selectedPhotoPickerItem,
            matching: .any(of: [.images, .videos])
        )
        .onChange(of: selectedPhotoPickerItem) { _, item in
            guard let item else { return }
            selectedPhotoPickerItem = nil
            Task { @MainActor in
                await handlePickedPhotoItem(item)
            }
        }
        #if canImport(UIKit)
        .sheet(isPresented: Binding(
            get: { pendingPhotoForEditor != nil },
            set: { if !$0 { pendingPhotoForEditor = nil } }
        )) {
            if let img = pendingPhotoForEditor {
                FabushiPhotoEditorView(
                    sourceImage: img,
                    onCancel: { pendingPhotoForEditor = nil },
                    onComplete: { _, jpegData in
                        pendingPhotoForEditor = nil
                        Task { @MainActor in
                            _ = await store.uploadAndSendAttachment(
                                chatId: chatId,
                                fileName: "photo-\(Int(Date().timeIntervalSince1970)).jpg",
                                mimeType: "image/jpeg",
                                data: jpegData,
                                topicId: store.activeTopicIdByChat[chatId],
                                contentBuilder: { uploaded in
                                    .photo(
                                        fileId: uploaded.fileId,
                                        caption: nil,
                                        width: uploaded.width ?? UInt32(img.size.width),
                                        height: uploaded.height ?? UInt32(img.size.height)
                                    )
                                },
                                router: router,
                                isCompact: horizontalSizeClass == .compact,
                                locale: theme.locale
                            )
                        }
                    }
                )
            }
        }
        .sheet(isPresented: $isDocumentPickerPresented) {
            FabushiDocumentPickerSheet(
                onPicked: { url in
                    isDocumentPickerPresented = false
                    Task { @MainActor in
                        await uploadPickedDocument(url: url)
                    }
                },
                onCancel: { isDocumentPickerPresented = false }
            )
        }
        .sheet(isPresented: $isCameraPresented) {
            FabushiCameraPickerSheet(
                onCaptured: { captured in
                    isCameraPresented = false
                    pendingPhotoForEditor = captured
                },
                onCancel: { isCameraPresented = false }
            )
        }
        #endif
        .sheet(isPresented: $isStickerPickerPresented) {
            FabushiStickersAndGifsPickerView(
                chatId: chatId,
                store: store,
                onInsertEmoji: { emoji in
                    composerText.append(emoji)
                },
                onDismiss: { isStickerPickerPresented = false }
            )
        }
        .sheet(isPresented: $isContextBadgePickerPresented) {
            FabushiContextBadgePickerSheetView(
                onAddBadge: { badge in
                    contextBadges.append(badge)
                    isContextBadgePickerPresented = false
                },
                onCancel: { isContextBadgePickerPresented = false }
            )
        }
        .sheet(isPresented: $isSendAsSheetPresented) {
            FabushiSendAsIdentitySheetView(
                chatId: chatId,
                store: store,
                onDismiss: { isSendAsSheetPresented = false }
            )
        }
        .sheet(isPresented: $isLocationSheetPresented) {
            locationShareSheet
        }
        .sheet(isPresented: $isPollCreatorPresented) {
            FabushiPollCreatorSheetView(
                chatId: chatId,
                store: store,
                onDismiss: { isPollCreatorPresented = false }
            )
        }
        .sheet(isPresented: $isTodoCreatorPresented) {
            FabushiTodoCreatorSheetView(
                chatId: chatId,
                store: store,
                onDismiss: { isTodoCreatorPresented = false }
            )
        }
        .sheet(isPresented: $isScheduleSheetPresented) {
            scheduleMessageSheet
        }
    }

    // MARK: - Main Input Row

    private var mainInputRow: some View {
        HStack(alignment: .bottom, spacing: 6) {
            // Attachment Menu Button
            Menu {
                Button {
                    isPhotoPickerPresented = true
                } label: {
                    Label(theme.messagingString(.attachPhotoVideoAction), systemImage: "photo.on.rectangle")
                }

                Button {
                    #if canImport(UIKit)
                    if UIImagePickerController.isSourceTypeAvailable(.camera) {
                        isCameraPresented = true
                    } else {
                        _ = router.showChatToastIfVisible(
                            theme.messagingString(.cameraUnavailableToast),
                            chatId: chatId,
                            coordinator: coordinator,
                            isCompact: horizontalSizeClass == .compact,
                            isError: false
                        )
                    }
                    #endif
                } label: {
                    Label(theme.messagingString(.attachCameraAction), systemImage: "camera")
                }

                Button {
                    isDocumentPickerPresented = true
                } label: {
                    Label(theme.messagingString(.attachDocumentAction), systemImage: "doc")
                }

                Button {
                    locationService.requestLocation()
                    isLocationSheetPresented = true
                } label: {
                    Label(theme.messagingString(.attachLocationAction), systemImage: "location")
                }

                Button {
                    isPollCreatorPresented = true
                } label: {
                    Label(theme.messagingString(.attachPollAction), systemImage: "chart.bar")
                }

                Button {
                    isTodoCreatorPresented = true
                } label: {
                    Label(theme.messagingString(.attachTodoAction), systemImage: "checklist")
                }

                Button {
                    isContextBadgePickerPresented = true
                } label: {
                    Label(theme.messagingString(.addContextBadgeAction), systemImage: "at")
                }

                Button {
                    showFormattingToolbar.toggle()
                } label: {
                    Label(theme.messagingString(.formatBoldAction), systemImage: "textformat")
                }
            } label: {
                Image(systemName: "plus.circle.fill")
                    .fabushiFont(.title3)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .accessibilityLabel(theme.messagingString(.composerAttachAction))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.attachButton)

            if !sendAsPeers.isEmpty {
                Button {
                    isSendAsSheetPresented = true
                } label: {
                    Image(systemName: "person.crop.circle")
                        .fabushiFont(.title3)
                        .foregroundStyle(theme.textSecondary)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.messagingString(.sendAsTitle))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.sendAsButton)
            }

            if chat?.isBot == true {
                Button {
                    router.navigate(to: .miniApp(id: nil), coordinator: coordinator)
                } label: {
                    Image(systemName: "square.grid.2x2")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.messagingString(.prestartBotOpenMiniAppAction))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.botMenuButton)
            }

            // Input Capsule (`FabushiIMESafeTextView`)
            HStack(alignment: .bottom, spacing: 4) {
                Button {
                    guard FabushiIMECompositionGuard.shouldAllowCommandOrMentionCommit(
                        hasMarkedText: isComposingMarkedText
                    ) else { return }
                    showSlashPopoverExplicitly.toggle()
                    if store.slashCommands.isEmpty {
                        Task { @MainActor in
                            await store.refreshSlashCommands()
                        }
                    }
                } label: {
                    Image(systemName: "slash.circle")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(showSlashPopoverExplicitly ? theme.accentColor : theme.textMuted)
                        .frame(width: 30, height: 34)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.messagingString(.slashCommandsTitle))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.botSlashButton)

                ZStack(alignment: .leading) {
                    if composerText.isEmpty {
                        Text(theme.messagingString(.composerPlaceholder))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.textMuted)
                            .padding(.leading, 8)
                            .allowsHitTesting(false)
                    }

                    #if canImport(UIKit)
                    FabushiIMESafeTextView(
                        text: $composerText,
                        isComposingMarkedText: $isComposingMarkedText,
                        measuredHeight: $measuredInputHeight,
                        placeholder: theme.messagingString(.composerPlaceholder),
                        textColor: UIColor(theme.textPrimary),
                        tintColor: UIColor(theme.accentColor),
                        accessibilityID: FabushiAccessibilityIDs.Composer.textField,
                        onSubmitReturn: {
                            submitPrimaryMessage(silent: false, scheduledAtMs: nil)
                        }
                    )
                    .frame(height: measuredInputHeight)
                    #else
                    TextField(theme.messagingString(.composerPlaceholder), text: $composerText, axis: .vertical)
                        .fabushiFont(.body)
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.textField)
                    #endif
                }
                .frame(maxWidth: .infinity)

                if !botReplyRows.isEmpty {
                    Button {
                        showReplyKeyboard.toggle()
                    } label: {
                        Image(systemName: showReplyKeyboard ? "keyboard.chevron.compact.down" : "square.grid.3x2")
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textSecondary)
                            .frame(width: 30, height: 34)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.botKeyboardToggle)
                }

                Button {
                    isStickerPickerPresented = true
                } label: {
                    Image(systemName: "face.smiling")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)
                        .frame(width: 30, height: 34)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(theme.messagingString(.stickerPickerTitle))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.stickerPickerButton)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(
                theme.surfaceInput,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous)
            )
            .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.inputWrap)

            // Trailing Send Button (with Silent & Schedule Context Menu) OR Hold-to-Record Voice Button
            if canSendText || editingMessage != nil {
                Button {
                    submitPrimaryMessage(silent: false, scheduledAtMs: nil)
                } label: {
                    Image(systemName: editingMessage != nil ? "checkmark.circle.fill" : "arrow.up.circle.fill")
                        .fabushiFont(.title2)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .contextMenu {
                    Button {
                        submitPrimaryMessage(silent: true, scheduledAtMs: nil)
                    } label: {
                        Label(theme.messagingString(.composerSendSilently), systemImage: "bell.slash")
                    }
                    Button {
                        scheduledDate = Date().addingTimeInterval(1800)
                        isScheduleSheetPresented = true
                    } label: {
                        Label(theme.messagingString(.composerScheduleMessage), systemImage: "calendar.badge.clock")
                    }
                }
                .accessibilityLabel(theme.messagingString(.composerSendAction))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.sendButton)
            } else {
                voiceHoldRecordButton
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    // MARK: - Real `AVFoundation` Voice Hold / Slide-to-Cancel / Lock Gesture

    private var voiceHoldRecordButton: some View {
        Image(systemName: "mic.fill")
            .fabushiFont(.title3)
            .foregroundStyle(theme.accentColor)
            .fabushiMinimumTouchTarget()
            .contentShape(Circle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        voiceDragOffset = value.translation
                        if !voiceRecorder.isRecording && !didCancelVoiceGesture {
                            triggerImpactHaptic(.medium)
                            Task { @MainActor in
                                await voiceRecorder.start()
                                if let err = voiceRecorder.errorMessage {
                                    validationErrorText = err
                                }
                            }
                        } else if voiceRecorder.isRecording && !isVoiceRecordingLocked {
                            if value.translation.width <= -72 {
                                didCancelVoiceGesture = true
                                triggerImpactHaptic(.light)
                                voiceRecorder.cancel()
                            } else if value.translation.height <= -56 {
                                isVoiceRecordingLocked = true
                                triggerImpactHaptic(.rigid)
                            }
                        }
                    }
                    .onEnded { _ in
                        defer {
                            voiceDragOffset = .zero
                            didCancelVoiceGesture = false
                        }
                        guard voiceRecorder.isRecording, !isVoiceRecordingLocked else { return }
                        finishAndSendVoiceRecording()
                    }
            )
            .accessibilityLabel(theme.messagingString(.composerVoiceRecordHint))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.voiceRecordButton)
    }

    private var voiceRecordingActiveRow: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(theme.destructiveColor)
                .frame(width: 10, height: 10)

            Text(formatElapsed(voiceRecorder.elapsedSeconds))
                .fabushiFont(.subheadline)
                .fontWeight(.semibold)
                .monospacedDigit()
                .foregroundStyle(theme.textPrimary)

            Text(
                isVoiceRecordingLocked
                    ? theme.messagingString(.composerRecordingLocked)
                    : theme.messagingString(.composerSlideToCancel)
            )
            .fabushiFont(.caption)
            .foregroundStyle(theme.textSecondary)
            .lineLimit(1)

            Spacer()

            Button(role: .destructive) {
                isVoiceRecordingLocked = false
                voiceRecorder.cancel()
            } label: {
                Image(systemName: "trash")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.destructiveColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.localized("hig.common.cancel"))

            Button {
                isVoiceRecordingLocked = false
                finishAndSendVoiceRecording()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .fabushiFont(.title2)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.messagingString(.composerSendAction))
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    // MARK: - Formatting Strip, Reply Banner & Bot Reply Keyboard

    private var formattingToolbarStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(FabushiComposerFormattingKind.allCases) { kind in
                    Button {
                        let applied = FabushiComposerFormattingHelper.apply(kind, to: composerText)
                        composerText = applied.text
                        if let entity = applied.entity {
                            pendingEntities.append(entity)
                        }
                    } label: {
                        Label(theme.messagingString(kind.stringKey), systemImage: kind.systemImage)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textPrimary)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(theme.surfaceInput, in: Capsule())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
    }

    private func replyOrEditBanner(
        icon: String,
        title: String,
        snippet: String,
        onDismiss: @escaping () -> Void
    ) -> some View {
        HStack(spacing: 10) {
            Image(systemName: icon)
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.accentColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .fabushiFont(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.accentColor)
                    .lineLimit(1)
                Text(snippet)
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .lineLimit(1)
            }
            Spacer()
            Button(action: onDismiss) {
                Image(systemName: "xmark.circle.fill")
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textMuted)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.localized("hig.common.close"))
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(theme.surfaceElevated)
    }

    private var botReplyKeyboardGrid: some View {
        VStack(spacing: 6) {
            ForEach(Array(botReplyRows.enumerated()), id: \.offset) { _, row in
                HStack(spacing: 6) {
                    ForEach(row, id: \.self) { label in
                        Button {
                            // `D10-07`: Bot Reply Keyboard taps MUST preserve unsent composer draft!
                            Task { @MainActor in
                                _ = try? await store.sendNonComposerMessage(
                                    chatId: chatId,
                                    text: label,
                                    topicId: store.activeTopicIdByChat[chatId]
                                )
                            }
                        } label: {
                            Text(label)
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textPrimary)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 10)
                                .background(
                                    theme.surfaceInput,
                                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                                )
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 10)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Composer.botReplyKeyboard)
    }

    // MARK: - Location & Schedule Sheets

    private var locationShareSheet: some View {
        NavigationStack {
            VStack(spacing: 16) {
                if locationService.loading {
                    ProgressView()
                } else if let coord = locationService.coordinate {
                    VStack(spacing: 8) {
                        Image(systemName: "mappin.and.ellipse")
                            .fabushiFont(.largeTitle)
                            .foregroundStyle(theme.accentColor)
                        Text(String(format: "%.5f, %.5f", coord.latitude, coord.longitude))
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textPrimary)
                    }
                    FabushiGlassPillButton(
                        title: theme.messagingString(.locationShareAction),
                        systemImage: "paperplane.fill",
                        accessibilityID: "fabushi.composer.sendLocationButton",
                        isProminent: true
                    ) {
                        let json = """
                        {"kind":"location","latitude":\(coord.latitude),"longitude":\(coord.longitude)}
                        """
                        isLocationSheetPresented = false
                        Task { @MainActor in
                            _ = try? await store.sendNonComposerMessage(
                                chatId: chatId,
                                text: String(format: "%.5f, %.5f", coord.latitude, coord.longitude),
                                content: .extended(kind: "location", rawJSON: json),
                                topicId: store.activeTopicIdByChat[chatId]
                            )
                        }
                    }
                } else {
                    Text(locationService.errorMessage ?? theme.messagingString(.locationCardTitle))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)
                        .multilineTextAlignment(.center)
                }
            }
            .padding(24)
            .navigationTitle(theme.messagingString(.locationCardTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        isLocationSheetPresented = false
                    }
                }
            }
        }
    }

    private var scheduleMessageSheet: some View {
        NavigationStack {
            Form {
                DatePicker(
                    theme.messagingString(.scheduleSendTitle),
                    selection: $scheduledDate,
                    in: Date().addingTimeInterval(60)...
                )
            }
            .navigationTitle(theme.messagingString(.scheduleSendTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel")) {
                        isScheduleSheetPresented = false
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.scheduleSendAction)) {
                        let ms = Int64(scheduledDate.timeIntervalSince1970 * 1000.0)
                        isScheduleSheetPresented = false
                        submitPrimaryMessage(silent: false, scheduledAtMs: ms)
                    }
                }
            }
        }
    }

    // MARK: - Actions & Draft / Media Helpers

    private func submitPrimaryMessage(silent: Bool, scheduledAtMs: Int64?) {
        guard !isComposingMarkedText else { return }
        let finalBody = FabushiSlashCommandCatalogHelper.composeMessageText(
            body: composerText,
            badges: contextBadges
        )
        guard !finalBody.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }

        if let editing = editingMessage {
            let entities = pendingEntities
            composerText = ""
            pendingEntities = []
            contextBadges = []
            Task { @MainActor in
                _ = await store.editMessage(
                    chatId: chatId,
                    messageId: editing.messageId,
                    newText: finalBody,
                    entities: entities,
                    router: router,
                    isCompact: horizontalSizeClass == .compact,
                    locale: theme.locale
                )
            }
            return
        }

        let entities = pendingEntities
        let activeTopic = store.activeTopicIdByChat[chatId]
        let isSlashCommand = FabushiIMECompositionGuard.activeSlashPrefix(
            text: finalBody.components(separatedBy: .whitespaces).first ?? "",
            hasMarkedText: false
        ) != nil

        composerText = ""
        pendingEntities = []
        contextBadges = []
        showSlashPopoverExplicitly = false
        draftSaveTask?.cancel()

        Task { @MainActor in
            if isSlashCommand {
                let botId: UInt64? = (chat?.isBot == true && chatId > 0) ? UInt64(chatId) : nil
                let result = await store.executeSlashCommand(
                    finalBody,
                    chatId: chatId,
                    botId: botId,
                    router: router,
                    isCompact: horizontalSizeClass == .compact,
                    locale: theme.locale
                )
                if result?.handled == true {
                    return
                }
            }

            _ = await store.sendComposerMessage(
                chatId: chatId,
                text: finalBody,
                entities: entities,
                replyTarget: store.replyTargetByChat[chatId],
                topicId: activeTopic,
                silent: silent,
                scheduledAtMs: scheduledAtMs,
                router: router,
                isCompact: horizontalSizeClass == .compact,
                locale: theme.locale
            )
        }
    }

    private func finishAndSendVoiceRecording() {
        let samples = voiceRecorder.waveformSamples
        let durationSeconds = UInt32(max(1, voiceRecorder.elapsedSeconds))
        guard let result = voiceRecorder.stop() else { return }
        let packedWaveform = WaveformCodec5Bit.encode(samples: samples)
        let activeTopic = store.activeTopicIdByChat[chatId]
        let fileName = result.url.lastPathComponent

        Task { @MainActor in
            _ = await store.uploadAndSendAttachment(
                chatId: chatId,
                fileName: fileName,
                mimeType: "audio/mp4",
                data: result.data,
                topicId: activeTopic,
                contentBuilder: { uploaded in
                    .voice(
                        FabushiVoiceItemPayload(
                            fileId: uploaded.fileId,
                            durationSeconds: uploaded.durationSeconds ?? durationSeconds,
                            waveform: packedWaveform,
                            mimeType: "audio/mp4",
                            sizeBytes: uploaded.sizeBytes
                        )
                    )
                },
                router: router,
                isCompact: horizontalSizeClass == .compact,
                locale: theme.locale
            )
        }
    }

    private func handlePickedPhotoItem(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self), !data.isEmpty else {
            return
        }
        #if canImport(UIKit)
        if let image = UIImage(data: data) {
            pendingPhotoForEditor = image
            return
        }
        #endif
        _ = await store.uploadAndSendAttachment(
            chatId: chatId,
            fileName: "video-\(Int(Date().timeIntervalSince1970)).mp4",
            mimeType: "video/mp4",
            data: data,
            topicId: store.activeTopicIdByChat[chatId],
            contentBuilder: { uploaded in
                .video(
                    fileId: uploaded.fileId,
                    duration: uploaded.durationSeconds ?? 0,
                    width: uploaded.width ?? 0,
                    height: uploaded.height ?? 0,
                    caption: nil
                )
            },
            router: router,
            isCompact: horizontalSizeClass == .compact,
            locale: theme.locale
        )
    }

    private func uploadPickedDocument(url: URL) async {
        let accessing = url.startAccessingSecurityScopedResource()
        defer {
            if accessing { url.stopAccessingSecurityScopedResource() }
        }
        guard let data = try? Data(contentsOf: url), !data.isEmpty else { return }
        let fileName = url.lastPathComponent
        let mimeType = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
        _ = await store.uploadAndSendAttachment(
            chatId: chatId,
            fileName: fileName,
            mimeType: mimeType,
            data: data,
            topicId: store.activeTopicIdByChat[chatId],
            contentBuilder: { uploaded in
                .document(
                    fileId: uploaded.fileId,
                    fileName: fileName,
                    mimeType: mimeType,
                    sizeBytes: uploaded.sizeBytes
                )
            },
            router: router,
            isCompact: horizontalSizeClass == .compact,
            locale: theme.locale
        )
    }

    private func syncComposerFromStoreDraft() {
        if let editing = editingMessage {
            composerText = editing.text
        } else {
            composerText = store.unsentDraftTextByChat[chatId] ?? store.draftsByChat[chatId]?.text ?? ""
        }
    }

    private func scheduleDebouncedDraftSave(text: String) {
        draftSaveTask?.cancel()
        let capturedChatId = chatId
        draftSaveTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 400_000_000)
            guard !Task.isCancelled else { return }
            await store.updateComposerDraft(chatId: capturedChatId, text: text, persistRemote: true)
        }
    }

    private func scheduleTypingPing(text: String) {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        guard typingTask == nil else { return }
        let capturedChatId = chatId
        typingTask = Task { @MainActor in
            _ = try? await store.client.sendTyping(chatId: capturedChatId, action: "typing")
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            typingTask = nil
        }
    }

    private func formatElapsed(_ seconds: Int) -> String {
        String(format: "%d:%02d", seconds / 60, seconds % 60)
    }

    #if canImport(UIKit)
    private func triggerImpactHaptic(_ style: UIImpactFeedbackGenerator.FeedbackStyle) {
        UIImpactFeedbackGenerator(style: style).impactOccurred()
    }
    #endif
}

// MARK: - Poll & Todo Creator Sheets

@MainActor
private struct FabushiPollCreatorSheetView: View {
    let chatId: Int64
    let store: FabushiMessagingStore
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var question: String = ""
    @State private var options: [String] = ["", ""]
    @State private var isAnonymous: Bool = true
    @State private var multipleChoice: Bool = false
    @State private var isQuiz: Bool = false
    @State private var correctOptionIndex: Int = 0
    @State private var isSending: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var validOptions: [String] {
        options.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(theme.messagingString(.pollQuestionPlaceholder), text: $question)
                        .fabushiFont(.body)
                }

                Section {
                    ForEach(options.indices, id: \.self) { idx in
                        TextField(
                            theme.messagingFormat(.pollOptionPlaceholderFormat, idx + 1),
                            text: Binding(
                                get: { options[idx] },
                                set: { options[idx] = $0 }
                            )
                        )
                        .fabushiFont(.body)
                    }
                    if options.count < 10 {
                        Button {
                            options.append("")
                        } label: {
                            Label(theme.messagingString(.pollAddOptionAction), systemImage: "plus.circle")
                        }
                    }
                }

                Section {
                    Toggle(theme.messagingString(.pollAnonymousToggle), isOn: $isAnonymous)
                    Toggle(theme.messagingString(.pollMultipleChoiceToggle), isOn: $multipleChoice)
                    Toggle(theme.messagingString(.pollQuizToggle), isOn: $isQuiz)
                    if isQuiz && !validOptions.isEmpty {
                        Picker(theme.messagingString(.pollQuizCorrectLabel), selection: $correctOptionIndex) {
                            ForEach(0..<validOptions.count, id: \.self) { idx in
                                Text(validOptions[idx]).tag(idx)
                            }
                        }
                    }
                }
            }
            .navigationTitle(theme.messagingString(.pollCreatorTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onDismiss)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.pollCreateAction)) {
                        guard !isSending else { return }
                        isSending = true
                        Task { @MainActor in
                            _ = await store.createPoll(
                                chatId: chatId,
                                question: question.trimmingCharacters(in: .whitespacesAndNewlines),
                                options: validOptions,
                                isAnonymous: isAnonymous,
                                multipleChoice: multipleChoice,
                                isQuiz: isQuiz,
                                correctOptionId: isQuiz ? UInt8(min(correctOptionIndex, validOptions.count - 1)) : nil,
                                topicId: store.activeTopicIdByChat[chatId]
                            )
                            isSending = false
                            onDismiss()
                        }
                    }
                    .disabled(
                        isSending
                            || question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            || validOptions.count < 2
                    )
                }
            }
        }
    }
}

@MainActor
private struct FabushiTodoCreatorSheetView: View {
    let chatId: Int64
    let store: FabushiMessagingStore
    let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var title: String = ""
    @State private var tasks: [String] = ["", ""]
    @State private var isSending: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var validTasks: [String] {
        tasks.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(theme.messagingString(.todoTitlePlaceholder), text: $title)
                        .fabushiFont(.body)
                }
                Section {
                    ForEach(tasks.indices, id: \.self) { idx in
                        TextField(
                            theme.messagingFormat(.todoItemPlaceholderFormat, idx + 1),
                            text: Binding(
                                get: { tasks[idx] },
                                set: { tasks[idx] = $0 }
                            )
                        )
                        .fabushiFont(.body)
                    }
                    if tasks.count < 12 {
                        Button {
                            tasks.append("")
                        } label: {
                            Label(theme.messagingString(.todoAddItemAction), systemImage: "plus.circle")
                        }
                    }
                }
            }
            .navigationTitle(theme.messagingString(.todoCreatorTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel"), action: onDismiss)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(theme.messagingString(.todoCreateAction)) {
                        guard !isSending else { return }
                        isSending = true
                        let items = validTasks.enumerated().map { idx, label in
                            FabushiTodoItem(id: "todo-\(idx)", title: label, completed: false)
                        }
                        let card = FabushiTodoCard(
                            id: UUID().uuidString,
                            title: title.trimmingCharacters(in: .whitespacesAndNewlines),
                            items: items,
                            completedCount: 0
                        )
                        Task { @MainActor in
                            _ = try? await store.sendNonComposerMessage(
                                chatId: chatId,
                                text: card.title,
                                content: .todo(card),
                                topicId: store.activeTopicIdByChat[chatId]
                            )
                            isSending = false
                            onDismiss()
                        }
                    }
                    .disabled(
                        isSending
                            || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            || validTasks.isEmpty
                    )
                }
            }
        }
    }
}
