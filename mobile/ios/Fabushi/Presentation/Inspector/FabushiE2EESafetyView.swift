import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - E2EE Safety Number Verification View (`#19`)

/// End-to-End Encryption Safety Number verification sheet wired to real pure-Rust `e2ee.identity`
/// and `e2ee.safetyFingerprint` RPCs, reusing `FabushiQRCodeRenderer` & `FabushiSecureClipboard`.
/// Explicitly respects `FabushiMessagingCapabilityBoundaries.e2eeServerPrekeyBoundaryPartial` without
/// fabricating remote server prekey enrollment or silently downgrading to plaintext.
@MainActor
public struct FabushiE2EESafetyView: View {
    public let chatId: Int64
    public let peerTitle: String
    public let remoteIdentityKeyB64: String?
    public let store: FabushiMessagingStore
    public let router: ShellNavigationRouter
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var safetyInfo: FabushiE2EESafetyInfo? = nil
    @State private var isLoading: Bool = false
    @State private var errorMessage: String? = nil
    @State private var copiedFeedback: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        chatId: Int64,
        peerTitle: String,
        remoteIdentityKeyB64: String? = nil,
        store: FabushiMessagingStore,
        router: ShellNavigationRouter,
        onDismiss: @escaping () -> Void
    ) {
        self.chatId = chatId
        self.peerTitle = peerTitle
        self.remoteIdentityKeyB64 = remoteIdentityKeyB64
        self.store = store
        self.router = router
        self.onDismiss = onDismiss
    }

    public var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    if isLoading {
                        ProgressView()
                            .padding(.top, 40)
                    } else if let info = safetyInfo {
                        safetyContentView(info)
                    } else if let errorMessage {
                        FabushiEmptyStateView(
                            title: theme.messagingString(.e2eeTitle),
                            systemImage: "lock.trianglebadge.exclamationmark",
                            descriptionText: errorMessage,
                            accessibilityID: "fabushi.e2ee.error"
                        )
                    }
                }
                .padding(20)
            }
            .background(theme.surfacePrimary)
            .navigationTitle(theme.messagingString(.e2eeSafetyNumbersTitle))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(theme.localized("hig.common.close")) {
                        store.overlayController.dismissE2EEVerification()
                        onDismiss()
                    }
                }
            }
            .task(id: chatId) {
                await loadSafetyNumbers()
            }
        }
    }

    private func safetyContentView(_ info: FabushiE2EESafetyInfo) -> some View {
        VStack(spacing: 18) {
            // Verification Status Pill
            HStack(spacing: 8) {
                Image(systemName: info.isVerified ? "checkmark.shield.fill" : "lock.shield")
                    .fabushiFont(.headline)
                    .foregroundStyle(info.isVerified ? theme.successColor : theme.warningColor)
                Text(
                    info.isVerified
                        ? theme.messagingString(.e2eeVerifiedStatus)
                        : theme.messagingString(.e2eeUnverifiedStatus)
                )
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(
                (info.isVerified ? theme.successColor : theme.warningColor).opacity(0.14),
                in: Capsule()
            )
            .accessibilityIdentifier(FabushiAccessibilityIDs.Stage.e2eeVerifiedBadge)

            // 4-Emoji Safety Number Grid
            let emojiTokens = Self.splitEmojiTokens(info.safetyEmojis)
            HStack(spacing: 16) {
                ForEach(Array(emojiTokens.enumerated()), id: \.offset) { _, token in
                    Text(token)
                        .fabushiFont(.largeTitle)
                        .frame(minWidth: 52, minHeight: 52)
                        .background(
                            theme.surfaceElevated,
                            in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                        )
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity)
            .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(theme.messagingString(.e2eeSafetyNumbersTitle)): \(info.safetyEmojis)")
            .accessibilityIdentifier(FabushiAccessibilityIDs.Inspector.e2eeSafetyEmojiGrid)

            Text(theme.messagingString(.e2eeSafetyNumbersHint))
                .fabushiFont(.footnote)
                .foregroundStyle(theme.textSecondary)
                .multilineTextAlignment(.center)

            // QR Code via CoreImage (`FabushiQRCodeRenderer`)
            #if canImport(UIKit)
            if let qrImage = FabushiQRCodeRenderer.renderQRCodeImage(from: info.qrPayload, scale: 8) {
                Image(uiImage: qrImage)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 184, height: 184)
                    .padding(14)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous))
                    .accessibilityLabel(theme.messagingString(.e2eeSafetyNumbersTitle))
            }
            #endif

            // Honest Remote Server Prekey Boundary Disclosure (`PARTIAL / STUB`)
            if FabushiMessagingCapabilityBoundaries.e2eeServerPrekeyBoundaryPartial && !info.peerEnrolledOnServer {
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "info.circle.fill")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.warningColor)
                    Text(theme.messagingString(.e2eePeerNotEnrolledNotice))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(12)
                .background(
                    theme.warningColor.opacity(0.12),
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )
            }

            // Key Fingerprint Details
            VStack(alignment: .leading, spacing: 10) {
                keyRow(
                    title: theme.messagingString(.e2eeLocalIdentityLabel),
                    value: info.localIdentityKeyB64
                )
                if let remoteKey = info.remoteIdentityKeyB64, !remoteKey.isEmpty {
                    Divider().overlay(theme.separatorHairline)
                    keyRow(
                        title: theme.messagingString(.e2eeRemoteIdentityLabel),
                        value: remoteKey
                    )
                }
            }
            .padding(14)
            .background(
                theme.surfaceElevated,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
            )

            // Actions
            VStack(spacing: 10) {
                FabushiGlassPillButton(
                    title: info.isVerified
                        ? theme.messagingString(.e2eeVerifiedStatus)
                        : theme.messagingString(.e2eeVerifyAction),
                    systemImage: info.isVerified ? "checkmark.circle.fill" : "checkmark.shield",
                    accessibilityID: FabushiAccessibilityIDs.Inspector.e2eeSafetyNumberButton,
                    isProminent: !info.isVerified
                ) {
                    let nextState = !info.isVerified
                    store.markE2EEVerified(chatId: chatId, verified: nextState)
                    safetyInfo?.isVerified = nextState
                    router.toastController.showStagePill(
                        nextState
                            ? theme.messagingString(.e2eeStartEncryptedNotice)
                            : theme.messagingString(.e2eeUnverifiedStatus),
                        chatId: chatId
                    )
                }

                FabushiGlassPillButton(
                    title: copiedFeedback
                        ? theme.messagingString(.messageCopiedPill)
                        : theme.messagingString(.e2eeCopyFingerprintAction),
                    systemImage: copiedFeedback ? "checkmark" : "doc.on.doc",
                    accessibilityID: "fabushi.e2ee.copyFingerprint",
                    isProminent: false
                ) {
                    _ = FabushiSecureClipboard.copySensitive(info.localIdentityKeyB64)
                    copiedFeedback = true
                    router.toastController.showStagePill(
                        theme.messagingString(.messageCopiedPill),
                        chatId: chatId
                    )
                }
            }
        }
    }

    private func keyRow(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .fabushiFont(.caption)
                .foregroundStyle(theme.textSecondary)
            Text(value)
                .fabushiFont(.monospacedCaption)
                .foregroundStyle(theme.textPrimary)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func loadSafetyNumbers() async {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        if let preloaded = store.overlayController.presentedSafetyInfo, preloaded.chatId == chatId {
            safetyInfo = preloaded
            return
        }
        do {
            safetyInfo = try await store.loadE2EESafetyInfo(
                chatId: chatId,
                peerTitle: peerTitle,
                remoteIdentityKeyB64: remoteIdentityKeyB64
            )
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    public static func splitEmojiTokens(_ raw: String) -> [String] {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return ["🔐", "🔑", "🛡️", "✨"] }
        let parts = trimmed.split(whereSeparator: \.isWhitespace).map(String.init)
        if parts.count >= 4 {
            return Array(parts.prefix(4))
        }
        let chars = trimmed.map(String.init).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        return chars.isEmpty ? ["🔐", "🔑", "🛡️", "✨"] : Array(chars.prefix(4))
    }
}
