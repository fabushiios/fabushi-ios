import SwiftUI

/// 160pt Apple HIG Full-Duplex AI Talk Mode VAD Orb (`Feature #29`).
///
/// Honors Reduce Motion (`theme.effectiveReduceMotion`), exposes `<60ms` instant barge-in,
/// hardware AEC status, on-device Apple Speech ASR capture, and live transcript feed.
@MainActor
public struct FabushiTalkModeOrbView: View {
    @Bindable public var viewModel: FabushiTalkModeViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }
    private var reduceMotion: Bool { theme.effectiveReduceMotion(systemReduceMotion: systemReduceMotion) }

    public init(viewModel: FabushiTalkModeViewModel) {
        self.viewModel = viewModel
    }

    private var orbAccentColor: Color {
        switch viewModel.state {
        case .listening:
            return theme.accentColor
        case .thinking:
            return theme.warningColor
        case .speaking:
            return theme.successColor
        case .interrupted:
            return theme.destructiveColor
        case .reconnecting:
            return theme.warningColor
        case .idle, .closed:
            return theme.textMuted
        }
    }

    private var orbScale: CGFloat {
        guard !reduceMotion, viewModel.state.isRunning else { return 1.0 }
        return 1.0 + CGFloat(min(0.16, viewModel.audioLevel * 0.22))
    }

    public var body: some View {
        ScrollView {
            VStack(spacing: 20) {
                // Header Status Pills
                HStack(spacing: 8) {
                    statusPill(
                        title: viewModel.isHardwareAecActive
                            ? FabushiPlatformStrings.text(.talkModeHardwareAecActive, locale: locale)
                            : FabushiPlatformStrings.text(.talkModeHardwareAecUnavailable, locale: locale),
                        systemImage: viewModel.isHardwareAecActive ? "waveform.badge.mic" : "waveform",
                        tint: viewModel.isHardwareAecActive ? theme.successColor : theme.textSecondary
                    )
                    statusPill(
                        title: FabushiPlatformStrings.text(.talkModeOfflineAsrLabel, locale: locale),
                        systemImage: "cpu",
                        tint: theme.accentColor
                    )
                }

                // 160pt Apple HIG Talk Mode VAD Orb
                ZStack {
                    if !reduceMotion && viewModel.state.isRunning {
                        Circle()
                            .fill(orbAccentColor.opacity(0.16))
                            .frame(width: 196, height: 196)
                            .scaleEffect(orbScale)
                            .animation(.easeInOut(duration: 0.22), value: viewModel.audioLevel)
                    }

                    Circle()
                        .fill(
                            RadialGradient(
                                colors: [
                                    orbAccentColor.opacity(0.92),
                                    orbAccentColor.opacity(0.55),
                                    theme.surfaceElevated
                                ],
                                center: .center,
                                startRadius: 12,
                                endRadius: 84
                            )
                        )
                        .frame(width: 160, height: 160)
                        .overlay {
                            Circle()
                                .stroke(theme.specularHighlight.opacity(0.6), lineWidth: 1.5)
                        }
                        .shadow(color: orbAccentColor.opacity(0.28), radius: 14, y: 6)

                    VStack(spacing: 6) {
                        Image(systemName: viewModel.state.isRunning ? "waveform.circle.fill" : "mic.circle.fill")
                            .fabushiFont(.largeTitle)
                            .foregroundStyle(.white)
                            .accessibilityHidden(true)
                        Text(viewModel.state.localizedTitle(locale: locale))
                            .fabushiFont(.footnote)
                            .foregroundStyle(.white)
                            .multilineTextAlignment(.center)
                            .lineLimit(2)
                    }
                    .padding(.horizontal, 14)
                }
                .frame(width: 204, height: 204)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(
                    "\(FabushiPlatformStrings.text(.talkModeTitle, locale: locale)): \(viewModel.state.localizedTitle(locale: locale))"
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Call.talkOrb)

                // Latency & Buffer Telemetry Strip
                if let bargeInMs = viewModel.lastBargeInElapsedMs {
                    Text(
                        FabushiPlatformStrings.text(
                            locale,
                            en: String(format: "Last barge-in: %.1f ms · Flushed %d chunks", bargeInMs, viewModel.lastFlushedChunks),
                            zhHans: String(format: "上次打断耗时：%.1f ms · 已清空 %d 个音频分片", bargeInMs, viewModel.lastFlushedChunks)
                        )
                    )
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textSecondary)
                }

                // Primary Talk Controls
                HStack(spacing: 12) {
                    if !viewModel.state.isRunning {
                        FabushiGlassPillButton(
                            title: FabushiPlatformStrings.text(.talkModeStartSession, locale: locale),
                            systemImage: "waveform.and.mic",
                            accessibilityID: "fabushi.call.talkStartButton",
                            isProminent: true
                        ) {
                            Task {
                                _ = await viewModel.startSession(remote: false)
                            }
                        }
                    } else {
                        FabushiGlassPillButton(
                            title: FabushiPlatformStrings.text(.talkModeBargeInAction, locale: locale),
                            systemImage: "bolt.horizontal.circle.fill",
                            accessibilityID: FabushiAccessibilityIDs.Call.talkBargeInButton,
                            isProminent: true
                        ) {
                            Task {
                                _ = await viewModel.triggerBargeIn()
                            }
                        }

                        if viewModel.state == .interrupted {
                            FabushiGlassPillButton(
                                title: FabushiPlatformStrings.text(.talkModeResumeListening, locale: locale),
                                systemImage: "ear.fill",
                                accessibilityID: "fabushi.call.talkResumeButton",
                                isProminent: false
                            ) {
                                Task {
                                    _ = await viewModel.resumeListening()
                                }
                            }
                        }

                        Button {
                            Task {
                                await viewModel.closeSession()
                            }
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.talkModeStopSession, locale: locale),
                                systemImage: "stop.circle.fill"
                            )
                            .fabushiFont(.subheadline)
                            .foregroundStyle(.white)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 10)
                            .background(theme.destructiveColor, in: Capsule())
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Call.talkStopButton)
                    }
                }

                // Voice Capture & Transcript Turn Input
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 8) {
                        TextField(
                            FabushiPlatformStrings.text(.talkModeSendTurnPlaceholder, locale: locale),
                            text: $viewModel.draftTranscript
                        )
                        .fabushiFont(.body)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(
                            theme.surfaceInput,
                            in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                        )
                        .submitLabel(.send)
                        .onSubmit {
                            Task { _ = await viewModel.sendTranscriptTurn() }
                        }

                        Button {
                            Task { await viewModel.toggleVoiceCaptureAndTranscribe(locale: locale) }
                        } label: {
                            Image(systemName: viewModel.isVoiceRecording ? "stop.fill" : "mic.fill")
                                .fabushiFont(.headline)
                                .foregroundStyle(viewModel.isVoiceRecording ? .white : theme.accentColor)
                                .frame(width: 44, height: 44)
                                .background(
                                    viewModel.isVoiceRecording ? theme.destructiveColor : theme.surfaceElevated,
                                    in: Circle()
                                )
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(
                            viewModel.isVoiceRecording
                                ? FabushiPlatformStrings.text(.talkModeVoiceStopAction, locale: locale)
                                : FabushiPlatformStrings.text(.talkModeVoiceCaptureAction, locale: locale)
                        )

                        Button {
                            Task { _ = await viewModel.sendTranscriptTurn() }
                        } label: {
                            Text(FabushiPlatformStrings.text(.talkModeSendTurnAction, locale: locale))
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.accentForegroundColor)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 10)
                                .background(theme.accentColor, in: Capsule())
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .disabled(viewModel.draftTranscript.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || viewModel.isBusy)
                    }

                    if let error = viewModel.lastErrorMessage, !error.isEmpty {
                        Text(error)
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
                .padding(14)
                .background(
                    theme.surfaceSecondary,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )

                // Live Transcript History
                if !viewModel.transcriptHistory.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(viewModel.transcriptHistory) { entry in
                            HStack {
                                if entry.isUser { Spacer(minLength: 32) }
                                Text(entry.text)
                                    .fabushiFont(.callout)
                                    .foregroundStyle(entry.isUser ? theme.accentForegroundColor : theme.textPrimary)
                                    .padding(.horizontal, 12)
                                    .padding(.vertical, 8)
                                    .background(
                                        entry.isUser ? theme.accentColor : theme.surfaceElevated,
                                        in: RoundedRectangle(cornerRadius: 12, style: .continuous)
                                    )
                                if !entry.isUser { Spacer(minLength: 32) }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .padding(16)
        }
        .background(theme.surfacePrimary)
    }

    private func statusPill(title: String, systemImage: String, tint: Color) -> some View {
        Label(title, systemImage: systemImage)
            .fabushiFont(.caption2)
            .foregroundStyle(tint)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(theme.surfaceSecondary, in: Capsule())
            .lineLimit(1)
    }
}
