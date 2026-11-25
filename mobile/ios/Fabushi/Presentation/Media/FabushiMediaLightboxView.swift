import AVKit
import Foundation
import Photos
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Reusable Native Activity Share Sheet

#if canImport(UIKit)
@MainActor
public struct FabushiActivityShareSheet: UIViewControllerRepresentable {
    public let items: [Any]

    public init(items: [Any]) {
        self.items = items
    }

    public func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    public func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}
#endif

// MARK: - Media Lightbox View (`#20`, `D10-08` Honest Download/Save Error Handling)

@MainActor
public struct FabushiMediaLightboxView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore
    public let target: FabushiMediaViewerTarget?
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var loadedData: Data? = nil
    @State private var isLoading: Bool = false
    @State private var errorMessage: String? = nil
    @State private var zoomScale: CGFloat = 1.0
    @State private var committedZoomScale: CGFloat = 1.0
    @State private var panOffset: CGSize = .zero
    @State private var committedPanOffset: CGSize = .zero
    @State private var isSharePresented: Bool = false
    @State private var isPhotoEditorPresented: Bool = false
    @State private var videoPlayer: AVPlayer? = nil
    @State private var tempVideoURL: URL? = nil

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var effectiveTarget: FabushiMediaViewerTarget? {
        target ?? store.overlayController.presentedMediaTarget
    }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil,
        target: FabushiMediaViewerTarget? = nil,
        onDismiss: @escaping () -> Void = {}
    ) {
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
        self.target = target
        self.onDismiss = onDismiss
    }

    public var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()

                if let activeTarget = effectiveTarget {
                    contentView(for: activeTarget)
                } else {
                    FabushiEmptyStateView(
                        title: theme.messagingString(.mediaLightboxTitle),
                        systemImage: "photo.on.rectangle.angled",
                        descriptionText: theme.messagingString(.inspectorNoSharedMedia),
                        accessibilityID: "fabushi.media.empty"
                    )
                }
            }
            .navigationTitle(effectiveTarget?.fileName ?? theme.messagingString(.mediaLightboxTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        cleanupVideo()
                        store.overlayController.dismissMediaLightbox()
                        onDismiss()
                    }
                }

                if let activeTarget = effectiveTarget, loadedData != nil {
                    ToolbarItemGroup(placement: .topBarTrailing) {
                        #if canImport(UIKit)
                        if !activeTarget.isVideo, UIImage(data: loadedData ?? Data()) != nil {
                            Button {
                                isPhotoEditorPresented = true
                            } label: {
                                Image(systemName: "slider.horizontal.3")
                                    .fabushiFont(.body)
                                    .fabushiMinimumTouchTarget()
                            }
                            .accessibilityLabel(theme.messagingString(.photoEditorTitle))
                        }
                        #endif

                        Button {
                            Task { @MainActor in
                                await saveMediaToPhotos(target: activeTarget)
                            }
                        } label: {
                            Image(systemName: "square.and.arrow.down")
                                .fabushiFont(.body)
                                .fabushiMinimumTouchTarget()
                        }
                        .accessibilityLabel(theme.messagingString(.mediaSaveAction))

                        Button {
                            isSharePresented = true
                        } label: {
                            Image(systemName: "square.and.arrow.up")
                                .fabushiFont(.body)
                                .fabushiMinimumTouchTarget()
                        }
                        .accessibilityLabel(theme.messagingString(.mediaShareAction))
                    }
                }
            }
            .task(id: effectiveTarget?.fileId) {
                await loadActiveMedia()
            }
            .onDisappear {
                cleanupVideo()
            }
            #if canImport(UIKit)
            .sheet(isPresented: $isSharePresented) {
                if let activeTarget = effectiveTarget, let shareItems = makeShareItems(target: activeTarget) {
                    FabushiActivityShareSheet(items: shareItems)
                }
            }
            .sheet(isPresented: $isPhotoEditorPresented) {
                if let data = loadedData, let image = UIImage(data: data) {
                    FabushiPhotoEditorView(
                        sourceImage: image,
                        onCancel: { isPhotoEditorPresented = false },
                        onComplete: { _, editedData in
                            loadedData = editedData
                            isPhotoEditorPresented = false
                            if let chatId = effectiveTarget?.chatId {
                                let name = "edited-\(effectiveTarget?.fileName ?? "photo.jpg")"
                                Task { @MainActor in
                                    _ = await store.uploadAndSendAttachment(
                                        chatId: chatId,
                                        fileName: name,
                                        mimeType: "image/jpeg",
                                        data: editedData,
                                        caption: effectiveTarget?.caption,
                                        contentBuilder: { uploaded in
                                            .photo(
                                                fileId: uploaded.fileId,
                                                caption: effectiveTarget?.caption,
                                                width: uploaded.width ?? 0,
                                                height: uploaded.height ?? 0
                                            )
                                        },
                                        router: router,
                                        isCompact: horizontalSizeClass == .compact,
                                        locale: theme.locale
                                    )
                                }
                            }
                        }
                    )
                }
            }
            #endif
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.mediaPlayerCard)
    }

    @ViewBuilder
    private func contentView(for activeTarget: FabushiMediaViewerTarget) -> some View {
        VStack(spacing: 0) {
            ZStack {
                if isLoading {
                    ProgressView()
                        .tint(.white)
                } else if let errorMessage {
                    VStack(spacing: 12) {
                        Image(systemName: "exclamationmark.triangle")
                            .fabushiFont(.title2)
                            .foregroundStyle(theme.warningColor)
                        Text(errorMessage)
                            .fabushiFont(.subheadline)
                            .foregroundStyle(.white)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 24)
                        FabushiGlassPillButton(
                            title: theme.localized("hig.common.retry"),
                            systemImage: "arrow.clockwise",
                            accessibilityID: "fabushi.media.retryButton",
                            isProminent: true
                        ) {
                            Task { @MainActor in
                                await loadActiveMedia()
                            }
                        }
                    }
                } else if activeTarget.isVideo, let videoPlayer {
                    VideoPlayer(player: videoPlayer)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    #if canImport(UIKit)
                    if let data = loadedData, let image = UIImage(data: data) {
                        Image(uiImage: image)
                            .resizable()
                            .scaledToFit()
                            .scaleEffect(zoomScale)
                            .offset(panOffset)
                            .gesture(
                                MagnificationGesture()
                                    .onChanged { value in
                                        zoomScale = min(5.0, max(1.0, committedZoomScale * value))
                                    }
                                    .onEnded { _ in
                                        committedZoomScale = zoomScale
                                        if zoomScale <= 1.05 {
                                            zoomScale = 1.0
                                            committedZoomScale = 1.0
                                            panOffset = .zero
                                            committedPanOffset = .zero
                                        }
                                    }
                            )
                            .simultaneousGesture(
                                DragGesture()
                                    .onChanged { value in
                                        guard zoomScale > 1.05 else { return }
                                        panOffset = CGSize(
                                            width: committedPanOffset.width + value.translation.width,
                                            height: committedPanOffset.height + value.translation.height
                                        )
                                    }
                                    .onEnded { _ in
                                        committedPanOffset = panOffset
                                    }
                            )
                            .onTapGesture(count: 2) {
                                if zoomScale > 1.05 {
                                    zoomScale = 1.0
                                    committedZoomScale = 1.0
                                    panOffset = .zero
                                    committedPanOffset = .zero
                                } else {
                                    zoomScale = 2.5
                                    committedZoomScale = 2.5
                                }
                            }
                            .accessibilityLabel(activeTarget.caption ?? activeTarget.fileName)
                    } else {
                        documentPreviewPlaceholder(for: activeTarget)
                    }
                    #else
                    documentPreviewPlaceholder(for: activeTarget)
                    #endif
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)

            if let caption = activeTarget.caption, !caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(caption)
                    .fabushiFont(.callout)
                    .foregroundStyle(.white)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .background(Color.black.opacity(0.68))
            }
        }
    }

    private func documentPreviewPlaceholder(for target: FabushiMediaViewerTarget) -> some View {
        VStack(spacing: 12) {
            Image(systemName: target.isVideo ? "film" : "doc.fill")
                .fabushiFont(.largeTitle)
                .foregroundStyle(.white)
            Text(target.fileName)
                .fabushiFont(.headline)
                .foregroundStyle(.white)
            Text(target.mimeType)
                .fabushiFont(.caption)
                .foregroundStyle(.white.opacity(0.75))
        }
        .padding(24)
    }

    private func loadActiveMedia() async {
        guard let activeTarget = effectiveTarget else { return }
        isLoading = true
        errorMessage = nil
        cleanupVideo()

        let targetChatId = activeTarget.chatId ?? store.activeChatId ?? 0
        if let bytes = await store.downloadAttachment(
            fileId: activeTarget.fileId,
            fileName: activeTarget.fileName,
            chatId: targetChatId,
            router: router,
            isCompact: horizontalSizeClass == .compact,
            locale: theme.locale
        ) {
            loadedData = bytes
            if activeTarget.isVideo {
                prepareVideoPlayer(bytes: bytes, fileName: activeTarget.fileName)
            }
            isLoading = false
        } else {
            isLoading = false
            errorMessage = theme.messagingFormat(.downloadFailedToastFormat, activeTarget.fileName)
        }
    }

    private func prepareVideoPlayer(bytes: Data, fileName: String) {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("fabushi-video-preview", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let ext = (fileName as NSString).pathExtension.isEmpty ? "mp4" : (fileName as NSString).pathExtension
        let url = dir.appendingPathComponent("\(UUID().uuidString).\(ext)")
        if (try? bytes.write(to: url, options: [.atomic])) != nil {
            tempVideoURL = url
            videoPlayer = AVPlayer(url: url)
        }
    }

    private func cleanupVideo() {
        videoPlayer?.pause()
        videoPlayer = nil
        if let tempVideoURL {
            try? FileManager.default.removeItem(at: tempVideoURL)
            self.tempVideoURL = nil
        }
    }

    /// `D10-08`: Authoritative Photos library save — NEVER displays `"Saved"` if data is missing
    /// or `PHPhotoLibrary` fails/denies permission!
    private func saveMediaToPhotos(target: FabushiMediaViewerTarget) async {
        let targetChatId = target.chatId ?? store.activeChatId ?? 0
        guard let data = loadedData, !data.isEmpty else {
            _ = router.showChatToastIfVisible(
                theme.messagingFormat(.downloadFailedToastFormat, target.fileName),
                chatId: targetChatId,
                coordinator: coordinator,
                isCompact: horizontalSizeClass == .compact,
                isError: true
            )
            return
        }

        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        guard status == .authorized || status == .limited else {
            _ = router.showChatToastIfVisible(
                theme.messagingFormat(.downloadFailedToastFormat, target.fileName),
                chatId: targetChatId,
                coordinator: coordinator,
                isCompact: horizontalSizeClass == .compact,
                isError: true
            )
            return
        }

        let resourceType: PHAssetResourceType = target.isVideo ? .video : .photo
        do {
            try await PHPhotoLibrary.shared().performChanges {
                let request = PHAssetCreationRequest.forAsset()
                request.addResource(with: resourceType, data: data, options: nil)
            }
            _ = router.showChatToastIfVisible(
                theme.messagingString(.mediaSavedToast),
                chatId: targetChatId,
                coordinator: coordinator,
                isCompact: horizontalSizeClass == .compact,
                isError: false
            )
        } catch {
            _ = router.showChatToastIfVisible(
                theme.messagingFormat(.downloadFailedToastFormat, target.fileName),
                chatId: targetChatId,
                coordinator: coordinator,
                isCompact: horizontalSizeClass == .compact,
                isError: true
            )
        }
    }

    #if canImport(UIKit)
    private func makeShareItems(target: FabushiMediaViewerTarget) -> [Any]? {
        guard let data = loadedData else { return nil }
        if !target.isVideo, let img = UIImage(data: data) {
            return [img]
        }
        let tempURL = FileManager.default.temporaryDirectory.appendingPathComponent(target.fileName)
        try? data.write(to: tempURL, options: [.atomic])
        return [tempURL]
    }
    #endif
}
