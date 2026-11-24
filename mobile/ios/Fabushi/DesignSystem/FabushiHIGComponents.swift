import Foundation
import Observation
import SwiftUI

// MARK: - Two-Layer Toast Models & Controller (Exact Upstream v1.6.8 / v1.6.9 Contract)

/// Layer 1 transient chat-stage pill notification (`fabushi.shell.toast.stagePill`).
public struct FabushiStageToast: Identifiable, Sendable, Equatable {
    public let id: UUID
    public let message: String
    public let scopeChatId: Int64?
    public let generation: UInt64

    public init(
        id: UUID = UUID(),
        message: String,
        scopeChatId: Int64?,
        generation: UInt64
    ) {
        self.id = id
        self.message = message
        self.scopeChatId = scopeChatId
        self.generation = generation
    }
}

/// Layer 2 trailing window toast banner (`fabushi.shell.toast.windowBanner`).
public struct FabushiWindowToast: Identifiable, Sendable, Equatable {
    public let id: UUID
    public let message: String
    public let isError: Bool
    public let scopeChatId: Int64?
    public let durationSeconds: TimeInterval

    public init(
        id: UUID = UUID(),
        message: String,
        isError: Bool,
        scopeChatId: Int64?,
        durationSeconds: TimeInterval = 3.0
    ) {
        self.id = id
        self.message = message
        self.isError = isError
        self.scopeChatId = scopeChatId
        self.durationSeconds = durationSeconds
    }
}

/// Manages the two-layer upstream `v1.6.9` toast architecture:
/// - Layer 1 (`stagePill`): generation-fenced, 2s default, tap-to-dismiss, cleared on chat switch.
/// - Layer 2 (`windowBanner` + FIFO `windowQueue`): ID-fenced, 3s default, explicit close CTA,
///   automatic error detection, consecutive spam deduplication, and chat-scoped active+queued purge.
@MainActor
@Observable
public final class FabushiToastController {
    nonisolated public static let defaultStageDurationSeconds: TimeInterval = 2.0
    nonisolated public static let defaultWindowDurationSeconds: TimeInterval = 3.0
    nonisolated public static let minimumDurationSeconds: TimeInterval = 0.08
    nonisolated public static let maxSingleLineSummaryLength: Int = 160

    public private(set) var stagePill: FabushiStageToast?
    public private(set) var stageGeneration: UInt64 = 0
    public private(set) var windowBanner: FabushiWindowToast?
    public private(set) var windowQueue: [FabushiWindowToast] = []

    @ObservationIgnored private var stageDismissTask: Task<Void, Never>?
    @ObservationIgnored private var windowDismissTask: Task<Void, Never>?
    @ObservationIgnored public var autoDismissEnabled: Bool

    public init(autoDismissEnabled: Bool = true) {
        self.autoDismissEnabled = autoDismissEnabled
    }

    /// Upstream `is_error_toast_text` detector (`PROVIDER_ERROR`, `Error:`, `失败`, `错误`, `failed`, `error`).
    nonisolated public static func isErrorText(_ message: String) -> Bool {
        let lower = message.lowercased()
        return message.contains("PROVIDER_ERROR")
            || lower.contains("error:")
            || message.contains("失败")
            || message.contains("错误")
            || lower.contains("failed")
            || lower.contains("error")
    }

    /// Collapses newlines and caps overly verbose error payloads to a clean single line.
    nonisolated public static func sanitizeSingleLine(
        _ message: String,
        maxLength: Int = maxSingleLineSummaryLength
    ) -> String {
        let collapsed = message
            .replacingOccurrences(of: "\r\n", with: " ")
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard collapsed.count > maxLength else { return collapsed }
        let prefixEnd = collapsed.index(collapsed.startIndex, offsetBy: max(1, maxLength - 1))
        return String(collapsed[..<prefixEnd]) + "…"
    }

    public func showStagePill(
        _ rawMessage: String,
        chatId: Int64? = nil,
        durationSeconds: TimeInterval = defaultStageDurationSeconds
    ) {
        let message = Self.sanitizeSingleLine(rawMessage)
        guard !message.isEmpty else { return }

        stageDismissTask?.cancel()
        stageGeneration &+= 1
        let capturedGeneration = stageGeneration
        stagePill = FabushiStageToast(
            message: message,
            scopeChatId: chatId,
            generation: capturedGeneration
        )

        guard autoDismissEnabled else { return }
        let clampedSeconds = max(durationSeconds, Self.minimumDurationSeconds)
        stageDismissTask = Task { @MainActor [weak self] in
            let nanos = UInt64(clampedSeconds * 1_000_000_000)
            try? await Task.sleep(nanoseconds: nanos)
            guard !Task.isCancelled, let self, self.stageGeneration == capturedGeneration else {
                return
            }
            self.stagePill = nil
        }
    }

    public func dismissStagePill() {
        stageDismissTask?.cancel()
        stageDismissTask = nil
        stageGeneration &+= 1
        stagePill = nil
    }

    public func showWindowToast(
        _ rawMessage: String,
        chatId: Int64? = nil,
        isError: Bool? = nil,
        durationSeconds: TimeInterval = defaultWindowDurationSeconds
    ) {
        let message = Self.sanitizeSingleLine(rawMessage)
        guard !message.isEmpty else { return }

        // v1.6.9 popup spam guard: suppress duplicate consecutive / in-flight (chatId, message).
        if let active = windowBanner,
           active.scopeChatId == chatId,
           active.message == message {
            return
        }
        if let lastQueued = windowQueue.last,
           lastQueued.scopeChatId == chatId,
           lastQueued.message == message {
            return
        }

        let resolvedDuration = durationSeconds <= 0
            ? Self.defaultWindowDurationSeconds
            : max(durationSeconds, Self.minimumDurationSeconds)

        let toast = FabushiWindowToast(
            message: message,
            isError: isError ?? Self.isErrorText(message),
            scopeChatId: chatId,
            durationSeconds: resolvedDuration
        )

        if windowBanner == nil {
            presentWindowToast(toast)
        } else {
            windowQueue.append(toast)
        }
    }

    /// Dismisses the active window toast (strictly ID-fenced when `id != nil`) and promotes
    /// the next FIFO queued toast if present.
    public func dismissWindowToast(id: UUID? = nil) {
        if let targetID = id, windowBanner?.id != targetID {
            return
        }
        windowDismissTask?.cancel()
        windowDismissTask = nil
        windowBanner = nil
        promoteNextQueuedWindowToast()
    }

    /// Synchronously clears Layer 1 stage pill AND purges BOTH active and queued Layer 2
    /// window toasts scoped to `chatId` (upstream `T5-06`/`T5-07`/`T5-13`/`T5-17`).
    public func clearToasts(forChatId chatId: Int64) {
        if let stage = stagePill, stage.scopeChatId == chatId {
            dismissStagePill()
        }

        windowQueue.removeAll { $0.scopeChatId == chatId }

        if let active = windowBanner, active.scopeChatId == chatId {
            windowDismissTask?.cancel()
            windowDismissTask = nil
            windowBanner = nil
            promoteNextQueuedWindowToast()
        }
    }

    /// Clears all Layer 1 and Layer 2 active and queued toasts (account switch / logout).
    public func clearAll() {
        stageDismissTask?.cancel()
        stageDismissTask = nil
        windowDismissTask?.cancel()
        windowDismissTask = nil
        stageGeneration &+= 1
        stagePill = nil
        windowBanner = nil
        windowQueue.removeAll()
    }

    private func presentWindowToast(_ toast: FabushiWindowToast) {
        windowDismissTask?.cancel()
        windowBanner = toast
        guard autoDismissEnabled else { return }

        let targetID = toast.id
        let duration = max(toast.durationSeconds, Self.minimumDurationSeconds)
        windowDismissTask = Task { @MainActor [weak self] in
            let nanos = UInt64(duration * 1_000_000_000)
            try? await Task.sleep(nanoseconds: nanos)
            guard !Task.isCancelled, let self else { return }
            self.dismissWindowToast(id: targetID)
        }
    }

    private func promoteNextQueuedWindowToast() {
        guard !windowQueue.isEmpty else { return }
        let next = windowQueue.removeFirst()
        presentWindowToast(next)
    }
}

/// Renders Layer 1 (`fabushi.shell.toast.stagePill`) and Layer 2 (`fabushi.shell.toast.windowBanner`)
/// cleanly above safe-area / bottom controls without overlapping input chrome.
@MainActor
public struct FabushiTwoLayerToastHost: View {
    public let controller: FabushiToastController

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(controller: FabushiToastController) {
        self.controller = controller
    }

    public var body: some View {
        VStack(spacing: 8) {
            if let stage = controller.stagePill {
                Button {
                    controller.dismissStagePill()
                } label: {
                    Text(stage.message)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.tooltipForeground)
                        .lineLimit(2)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .background(theme.tooltipBackground, in: Capsule())
                        .overlay(
                            Capsule().strokeBorder(theme.hairlineBorder, lineWidth: 0.5)
                        )
                }
                .buttonStyle(.plain)
                .fabushiMinimumTouchTarget()
                .accessibilityLabel(stage.message)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.toastStagePill)
            }

            if let banner = controller.windowBanner {
                HStack(spacing: 10) {
                    if banner.isError {
                        Image(systemName: "exclamationmark.circle.fill")
                            .foregroundStyle(theme.destructiveColor)
                            .accessibilityHidden(true)
                    } else {
                        Image(systemName: "info.circle.fill")
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)
                    }

                    Text(banner.message)
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textPrimary)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    Button {
                        controller.dismissWindowToast(id: banner.id)
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized("hig.common.close"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.toastWindowBannerClose)
                }
                .padding(.leading, 14)
                .padding(.trailing, 4)
                .padding(.vertical, 6)
                .frame(maxWidth: 480)
                .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius)
                .overlay(
                    RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous)
                        .strokeBorder(
                            banner.isError ? theme.destructiveColor.opacity(0.65) : theme.hairlineBorder,
                            lineWidth: banner.isError ? 1.0 : 0.5
                        )
                )
                .accessibilityElement(children: .contain)
                .accessibilityLabel(banner.message)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.toastWindowBanner)
            }
        }
        .padding(.horizontal, 16)
        .animation(
            theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)
                ? nil
                : .easeInOut(duration: 0.2),
            value: controller.stagePill?.id
        )
        .animation(
            theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion)
                ? nil
                : .easeInOut(duration: 0.2),
            value: controller.windowBanner?.id
        )
    }
}

// MARK: - Reusable Apple HIG Components

/// Native Apple HIG pill button guaranteeing `>= 44×44pt` hit target, VoiceOver label, and Liquid Glass chrome.
@MainActor
public struct FabushiGlassPillButton: View {
    public let title: String
    public let systemImage: String?
    public let accessibilityID: String
    public let isProminent: Bool
    public let action: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        title: String,
        systemImage: String? = nil,
        accessibilityID: String,
        isProminent: Bool = false,
        action: @escaping () -> Void
    ) {
        self.title = title
        self.systemImage = systemImage
        self.accessibilityID = accessibilityID
        self.isProminent = isProminent
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if let systemImage {
                    Image(systemName: systemImage)
                        .accessibilityHidden(true)
                }
                Text(title)
                    .fabushiFont(.callout)
                    .fontWeight(.semibold)
                    .lineLimit(2)
                    .minimumScaleFactor(0.85)
            }
            .foregroundStyle(isProminent ? theme.accentForegroundColor : theme.textPrimary)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .fabushiMinimumTouchTarget()
            .background {
                if isProminent {
                    RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                        .fill(theme.accentColor)
                }
            }
            .modifier(OptionalGlassModifier(enabled: !isProminent))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityIdentifier(accessibilityID)
    }
}

@MainActor
private struct OptionalGlassModifier: ViewModifier {
    let enabled: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        if enabled {
            content.fabushiLiquidGlass(
                cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius,
                isInteractive: true
            )
        } else {
            content
        }
    }
}

/// Deterministic 7-color peer avatar badge with Dynamic Type scaling, optional online indicator, and bot badge.
@MainActor
public struct FabushiAvatarBadge: View {
    public let peerId: Int64
    public let title: String
    public let isOnline: Bool
    public let isBot: Bool

    @ScaledMetric(relativeTo: .title3) private var avatarDiameter: CGFloat = 44
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(peerId: Int64, title: String, isOnline: Bool = false, isBot: Bool = false) {
        self.peerId = peerId
        self.title = title
        self.isOnline = isOnline
        self.isBot = isBot
    }

    private var initials: String {
        let parts = title
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .split(whereSeparator: { $0.isWhitespace })
        if parts.count >= 2, let first = parts[0].first, let second = parts[1].first {
            return String([first, second]).uppercased()
        }
        if let first = title.trimmingCharacters(in: .whitespacesAndNewlines).first {
            return String(first).uppercased()
        }
        return "?"
    }

    public var body: some View {
        let gradientPair = FabushiPeerColorPalette.avatarGradientRGB(peerId: peerId)
        let clampedSize = min(max(avatarDiameter, 36), 64)

        ZStack(alignment: .bottomTrailing) {
            Circle()
                .fill(
                    LinearGradient(
                        colors: [
                            Color(fabushiHex: gradientPair.top),
                            Color(fabushiHex: gradientPair.bottom),
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .frame(width: clampedSize, height: clampedSize)
                .overlay {
                    Text(initials)
                        .fabushiFont(.headline)
                        .foregroundStyle(Color.white)
                }

            if isBot {
                Image(systemName: "sparkles")
                    .fabushiFont(.caption2)
                    .foregroundStyle(Color.white)
                    .padding(3)
                    .background(theme.accentColor, in: Circle())
                    .accessibilityIdentifier(FabushiAccessibilityIDs.ChatList.botAvatarBadge)
            } else if isOnline {
                Circle()
                    .fill(theme.successColor)
                    .frame(width: 11, height: 11)
                    .overlay(Circle().stroke(theme.surfacePrimary, lineWidth: 2))
            }
        }
        .accessibilityHidden(true)
    }
}

/// Dynamic Type section header aligned with Apple HIG inset grouped lists.
@MainActor
public struct FabushiSectionHeader: View {
    public let title: String

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(_ title: String) {
        self.title = title
    }

    public var body: some View {
        Text(title)
            .fabushiFont(.subheadline)
            .fontWeight(.semibold)
            .foregroundStyle(theme.textSecondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityAddTraits(.isHeader)
    }
}

/// Reusable native `ContentUnavailableView` wrapper styled with `FabushiTheme`.
@MainActor
public struct FabushiEmptyStateView: View {
    public let title: String
    public let systemImage: String
    public let descriptionText: String
    public let accessibilityID: String

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        title: String,
        systemImage: String,
        descriptionText: String,
        accessibilityID: String
    ) {
        self.title = title
        self.systemImage = systemImage
        self.descriptionText = descriptionText
        self.accessibilityID = accessibilityID
    }

    public var body: some View {
        ContentUnavailableView {
            Label(title, systemImage: systemImage)
                .foregroundStyle(theme.textPrimary)
        } description: {
            Text(descriptionText)
                .fabushiFont(.body)
                .foregroundStyle(theme.textSecondary)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title). \(descriptionText)")
        .accessibilityIdentifier(accessibilityID)
    }
}

/// Fail-closed startup / offline unavailable surface (`fabushi.shell.offlineUnavailableView`)
/// with an explicit `>= 44×44pt` Retry action (`fabushi.shell.offlineRetryButton`).
@MainActor
public struct FabushiStartupRetryView: View {
    public let errorMessage: String
    public let isRetrying: Bool
    public let onRetry: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        errorMessage: String,
        isRetrying: Bool = false,
        onRetry: @escaping () -> Void
    ) {
        self.errorMessage = errorMessage
        self.isRetrying = isRetrying
        self.onRetry = onRetry
    }

    public var body: some View {
        ContentUnavailableView {
            Label(
                theme.localized("hig.offline.title"),
                systemImage: "wifi.exclamationmark"
            )
            .foregroundStyle(theme.textPrimary)
        } description: {
            VStack(spacing: 8) {
                Text(theme.localized("hig.offline.description"))
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textSecondary)
                    .multilineTextAlignment(.center)

                if !errorMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(FabushiToastController.sanitizeSingleLine(errorMessage, maxLength: 200))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textMuted)
                        .multilineTextAlignment(.center)
                }
            }
        } actions: {
            FabushiGlassPillButton(
                title: isRetrying
                    ? theme.localized("hig.status.connecting")
                    : theme.localized("hig.common.retry"),
                systemImage: "arrow.clockwise",
                accessibilityID: FabushiAccessibilityIDs.Shell.offlineRetryButton,
                isProminent: true,
                action: onRetry
            )
            .disabled(isRetrying)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.surfacePrimary)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(theme.localized("hig.offline.title"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.offlineUnavailableView)
    }
}

/// Connection status pill (`fabushi.shell.connectionStatusPill`) reflecting live online / WS status.
@MainActor
public struct FabushiConnectionStatusPill: View {
    public let isOnline: Bool
    public let wsStatus: String

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(isOnline: Bool, wsStatus: String) {
        self.isOnline = isOnline
        self.wsStatus = wsStatus
    }

    private var displayTitle: String {
        if !isOnline {
            return theme.localized("hig.status.waitingNetwork")
        }
        let lower = wsStatus.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch lower {
        case "connected", "online":
            return theme.localized("hig.status.connected")
        case "connecting", "reconnecting":
            return theme.localized("hig.status.connecting")
        case "updating", "syncing":
            return theme.localized("hig.status.updating")
        default:
            return theme.localized("hig.status.ready")
        }
    }

    private var indicatorColor: Color {
        if !isOnline {
            return theme.warningColor
        }
        let lower = wsStatus.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch lower {
        case "connected", "online":
            return theme.successColor
        case "connecting", "reconnecting", "updating", "syncing":
            return theme.accentColor
        default:
            return theme.textSecondary
        }
    }

    public var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(indicatorColor)
                .frame(width: 8, height: 8)
                .accessibilityHidden(true)
            Text(displayTitle)
                .fabushiFont(.caption)
                .fontWeight(.medium)
                .foregroundStyle(theme.textSecondary)
                .lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(theme.surfaceInput, in: Capsule())
        .overlay(Capsule().strokeBorder(theme.hairlineBorder, lineWidth: 0.5))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(displayTitle)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.connectionStatusPill)
    }
}

/// Persistent non-auto-dismissing AI provider error card (`fabushi.shell.providerErrorCard`)
/// with an explicit `>= 44×44pt` CTA (`fabushi.shell.providerErrorCard.openAiProviders`).
@MainActor
public struct FabushiProviderErrorCard: View {
    public let detail: String
    public let onOpenAIProviders: () -> Void
    public let onDismiss: (() -> Void)?

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        detail: String,
        onOpenAIProviders: @escaping () -> Void,
        onDismiss: (() -> Void)? = nil
    ) {
        self.detail = detail
        self.onOpenAIProviders = onOpenAIProviders
        self.onDismiss = onDismiss
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(theme.destructiveColor)
                    .accessibilityHidden(true)
                Text(theme.localized("hig.providerError.title"))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                Spacer(minLength: 4)
                if let onDismiss {
                    Button(action: onDismiss) {
                        Image(systemName: "xmark")
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized("hig.common.close"))
                }
            }

            Text(FabushiToastController.sanitizeSingleLine(detail, maxLength: 200))
                .fabushiFont(.footnote)
                .foregroundStyle(theme.textSecondary)

            FabushiGlassPillButton(
                title: theme.localized("hig.providerError.openProviders"),
                systemImage: "slider.horizontal.3",
                accessibilityID: FabushiAccessibilityIDs.Shell.providerErrorOpenAIProviders,
                isProminent: true,
                action: onOpenAIProviders
            )
        }
        .padding(14)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius)
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(theme.destructiveColor.opacity(0.6), lineWidth: 1.0)
        )
        .accessibilityElement(children: .contain)
        .accessibilityLabel(theme.localized("hig.providerError.title"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.providerErrorCard)
    }
}
