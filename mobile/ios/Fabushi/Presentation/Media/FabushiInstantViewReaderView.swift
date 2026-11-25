import Foundation
import SafariServices
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Pure Swift Instant View Article Parser

public enum FabushiInstantViewParser: Sendable {
    /// Parses Markdown or lightweight HTML into structured `FabushiInstantViewDocument` blocks.
    public static func parse(
        url: String,
        fallbackTitle: String,
        rawText: String
    ) -> FabushiInstantViewDocument {
        let host = URL(string: url)?.host ?? url
        let lines = rawText
            .replacingOccurrences(of: "\r\n", with: "\n")
            .components(separatedBy: "\n")

        var title = fallbackTitle
        var blocks: [FabushiInstantViewBlock] = []
        var inCodeBlock = false
        var codeLang: String? = nil
        var codeBuffer: [String] = []

        for line in lines {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("```") {
                if inCodeBlock {
                    blocks.append(.code(language: codeLang, content: codeBuffer.joined(separator: "\n")))
                    codeBuffer.removeAll()
                    codeLang = nil
                    inCodeBlock = false
                } else {
                    inCodeBlock = true
                    let lang = String(trimmed.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                    codeLang = lang.isEmpty ? nil : lang
                }
                continue
            }

            if inCodeBlock {
                codeBuffer.append(line)
                continue
            }

            if trimmed.isEmpty {
                continue
            }
            if trimmed == "---" || trimmed == "***" {
                blocks.append(.divider)
            } else if trimmed.hasPrefix("# ") {
                let text = String(trimmed.dropFirst(2)).trimmingCharacters(in: .whitespaces)
                if title.isEmpty || title == fallbackTitle {
                    title = text
                } else {
                    blocks.append(.heading(text))
                }
            } else if trimmed.hasPrefix("## ") || trimmed.hasPrefix("### ") {
                let stripped = trimmed.drop(while: { $0 == "#" }).trimmingCharacters(in: .whitespaces)
                blocks.append(.heading(stripped))
            } else if trimmed.hasPrefix("> ") {
                blocks.append(.quote(String(trimmed.dropFirst(2)).trimmingCharacters(in: .whitespaces)))
            } else {
                blocks.append(.paragraph(stripBasicHTMLTags(trimmed)))
            }
        }

        if inCodeBlock && !codeBuffer.isEmpty {
            blocks.append(.code(language: codeLang, content: codeBuffer.joined(separator: "\n")))
        }

        if blocks.isEmpty {
            blocks.append(.paragraph(rawText.trimmingCharacters(in: .whitespacesAndNewlines)))
        }

        return FabushiInstantViewDocument(
            url: url,
            siteName: host,
            title: title.isEmpty ? host : title,
            blocks: blocks
        )
    }

    private static func stripBasicHTMLTags(_ input: String) -> String {
        guard input.contains("<"), input.contains(">") else { return input }
        return input.replacingOccurrences(of: "<[^>]+>", with: "", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

#if canImport(UIKit)
@MainActor
private struct FabushiSafariSheetView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> SFSafariViewController {
        SFSafariViewController(url: url)
    }

    func updateUIViewController(_ uiViewController: SFSafariViewController, context: Context) {}
}
#endif

// MARK: - Apple HIG Instant View Article Reader (`#20`, `D10-07` Draft-Preserving Share)

@MainActor
public struct FabushiInstantViewReaderView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let store: FabushiMessagingStore
    public let document: FabushiInstantViewDocument?
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var dynamicTypeStep: Int = 1 // 0: compact, 1: standard, 2: large
    @State private var isShareToChatPresented: Bool = false
    @State private var isSafariPresented: Bool = false
    @State private var isSystemSharePresented: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private var activeDocument: FabushiInstantViewDocument? {
        document ?? store.overlayController.presentedInstantView
    }

    private var resolvedDynamicTypeSize: DynamicTypeSize {
        switch dynamicTypeStep {
        case 0: return .medium
        case 2: return .xxLarge
        default: return .large
        }
    }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        store: FabushiMessagingStore? = nil,
        document: FabushiInstantViewDocument? = nil,
        onDismiss: @escaping () -> Void = {}
    ) {
        self.coordinator = coordinator
        self.router = router
        self.store = store ?? FabushiMessagingStoreRegistry.shared.store(for: coordinator)
        self.document = document
        self.onDismiss = onDismiss
    }

    public var body: some View {
        NavigationStack {
            Group {
                if let doc = activeDocument {
                    articleBodyView(doc)
                } else {
                    FabushiEmptyStateView(
                        title: theme.messagingString(.instantViewTitle),
                        systemImage: "doc.richtext",
                        descriptionText: theme.messagingString(.inspectorNoSharedMedia),
                        accessibilityID: "fabushi.instantView.empty"
                    )
                }
            }
            .background(theme.surfacePrimary)
            .navigationTitle(activeDocument?.siteName ?? theme.messagingString(.instantViewTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        store.overlayController.dismissInstantView()
                        onDismiss()
                    }
                }
                if let doc = activeDocument {
                    ToolbarItemGroup(placement: .topBarTrailing) {
                        Button {
                            dynamicTypeStep = (dynamicTypeStep + 1) % 3
                        } label: {
                            Image(systemName: "textformat.size")
                                .fabushiFont(.body)
                                .fabushiMinimumTouchTarget()
                        }
                        .accessibilityLabel(theme.messagingString(.instantViewTextSizeLabel))

                        Button {
                            isShareToChatPresented = true
                        } label: {
                            Image(systemName: "arrowshape.turn.up.right")
                                .fabushiFont(.body)
                                .fabushiMinimumTouchTarget()
                        }
                        .accessibilityLabel(theme.messagingString(.instantViewShareAction))

                        if URL(string: doc.url) != nil {
                            Button {
                                isSafariPresented = true
                            } label: {
                                Image(systemName: "safari")
                                    .fabushiFont(.body)
                                    .fabushiMinimumTouchTarget()
                            }
                            .accessibilityLabel(theme.messagingString(.instantViewOpenInSafariAction))
                        }
                    }
                }
            }
            .sheet(isPresented: $isShareToChatPresented) {
                if let doc = activeDocument {
                    shareToChatSheet(doc)
                }
            }
            #if canImport(UIKit)
            .sheet(isPresented: $isSafariPresented) {
                if let urlStr = activeDocument?.url, let url = URL(string: urlStr) {
                    FabushiSafariSheetView(url: url)
                }
            }
            .sheet(isPresented: $isSystemSharePresented) {
                if let doc = activeDocument {
                    FabushiActivityShareSheet(items: [doc.title, doc.url])
                }
            }
            #endif
        }
        .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.instantViewCard)
    }

    private func articleBodyView(_ doc: FabushiInstantViewDocument) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(doc.title)
                    .fabushiFont(.title2)
                    .foregroundStyle(theme.textPrimary)

                HStack(spacing: 8) {
                    Text(doc.siteName)
                        .fabushiFont(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(theme.accentColor)
                    if let author = doc.author, !author.isEmpty {
                        Text("• \(author)")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    if let publishedAt = doc.publishedAt, !publishedAt.isEmpty {
                        Text("• \(publishedAt)")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textMuted)
                    }
                }

                Divider().overlay(theme.separatorHairline)

                ForEach(Array(doc.blocks.enumerated()), id: \.offset) { _, block in
                    renderBlock(block)
                }

                Divider().overlay(theme.separatorHairline)

                HStack(spacing: 12) {
                    FabushiGlassPillButton(
                        title: theme.messagingString(.instantViewShareAction),
                        systemImage: "paperplane",
                        accessibilityID: "fabushi.instantView.shareToChat",
                        isProminent: true
                    ) {
                        isShareToChatPresented = true
                    }

                    FabushiGlassPillButton(
                        title: theme.messagingString(.mediaShareAction),
                        systemImage: "square.and.arrow.up",
                        accessibilityID: "fabushi.instantView.systemShare",
                        isProminent: false
                    ) {
                        isSystemSharePresented = true
                    }
                }
            }
            .padding(20)
            .dynamicTypeSize(resolvedDynamicTypeSize)
        }
    }

    @ViewBuilder
    private func renderBlock(_ block: FabushiInstantViewBlock) -> some View {
        switch block {
        case .heading(let heading):
            Text(heading)
                .fabushiFont(.title3)
                .foregroundStyle(theme.textPrimary)
        case .paragraph(let text):
            Text(text)
                .fabushiFont(.body)
                .foregroundStyle(theme.textPrimary)
                .textSelection(.enabled)
        case .quote(let quote):
            HStack(alignment: .top, spacing: 10) {
                Capsule()
                    .fill(theme.accentColor)
                    .frame(width: 3)
                Text(quote)
                    .fabushiFont(.callout)
                    .italic()
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(.vertical, 4)
        case .code(let language, let content):
            VStack(alignment: .leading, spacing: 6) {
                if let language, !language.isEmpty {
                    Text(language.uppercased())
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textMuted)
                }
                ScrollView(.horizontal, showsIndicators: false) {
                    Text(content)
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(theme.textPrimary)
                        .textSelection(.enabled)
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                theme.surfaceElevated,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
            )
        case .image(let url, let caption):
            VStack(alignment: .leading, spacing: 6) {
                RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                    .fill(theme.surfaceElevated)
                    .frame(height: 160)
                    .overlay(
                        VStack(spacing: 6) {
                            Image(systemName: "photo")
                                .fabushiFont(.title2)
                                .foregroundStyle(theme.textSecondary)
                            Text(url)
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                                .lineLimit(1)
                                .padding(.horizontal, 12)
                        }
                    )
                if let caption, !caption.isEmpty {
                    Text(caption)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
            }
        case .divider:
            Divider().overlay(theme.separatorHairline)
        }
    }

    private func shareToChatSheet(_ doc: FabushiInstantViewDocument) -> some View {
        NavigationStack {
            List(store.chats) { chat in
                Button {
                    // `D10-07`: Share via `sendNonComposerMessage` so target chat's unsent draft is preserved!
                    Task { @MainActor in
                        _ = try? await store.sendNonComposerMessage(
                            chatId: chat.chatId,
                            text: "\(doc.title)\n\(doc.url)"
                        )
                        isShareToChatPresented = false
                    }
                } label: {
                    HStack(spacing: 12) {
                        FabushiAvatarBadge(
                            peerId: chat.chatId,
                            title: chat.title,
                            isOnline: false,
                            isBot: chat.isBot
                        )
                        Text(chat.title)
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.textPrimary)
                        Spacer()
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
            }
            .listStyle(.plain)
            .navigationTitle(theme.messagingString(.forwardPickerTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.cancel")) {
                        isShareToChatPresented = false
                    }
                }
            }
        }
    }
}
