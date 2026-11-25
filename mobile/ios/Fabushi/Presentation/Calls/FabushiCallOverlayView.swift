import SwiftUI

/// Unified Apple HIG Active Call Surface (`Feature #29`) wrapping existing Stratum A
/// `HumanCallVideoStage`, `HumanCallBroadcastPicker`, 4-emoji E2EE fingerprint verification,
/// permission & ReplayKit failure banners, and honest `BLK-02` SFU/NAT disclosure.
@MainActor
public struct FabushiCallOverlayView: View {
    @Bindable public var controller: FabushiCallSessionController

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(controller: FabushiCallSessionController) {
        self.controller = controller
    }

    public var body: some View {
        VStack(spacing: 16) {
            // Honest BLK-02 WebRTC NAT/SFU & Two-Device Disclosure Banner (ALWAYS visible)
            disclosureCard(
                title: FabushiPlatformStrings.text(.callsSfuDisclosureTitle, locale: locale),
                body: FabushiPlatformStrings.text(.callsSfuDisclosureBody, locale: locale),
                systemImage: "network.badge.shield.half.filled",
                tint: theme.warningColor
            )
            .accessibilityIdentifier(FabushiAccessibilityIDs.Call.sfuDisclosureBanner)

            // Permission Denied Banner (`simulateCallPermissionDenied` or denied AV authorization)
            if controller.isPermissionDenied {
                VStack(alignment: .leading, spacing: 10) {
                    Label(
                        FabushiPlatformStrings.text(.callsPermissionDeniedTitle, locale: locale),
                        systemImage: "exclamationmark.triangle.fill"
                    )
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.destructiveColor)

                    Text(FabushiPlatformStrings.text(.callsPermissionDeniedBody, locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)

                    Button {
                        controller.openSystemSettings()
                    } label: {
                        Text(FabushiPlatformStrings.text(.callsOpenSystemSettings, locale: locale))
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.accentForegroundColor)
                            .padding(.horizontal, 14)
                            .padding(.vertical, 8)
                            .background(theme.accentColor, in: Capsule())
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Call.permissionDeniedOpenSettings)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(14)
                .background(
                    theme.surfaceSecondary,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )
                .overlay {
                    RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                        .stroke(theme.destructiveColor.opacity(0.45), lineWidth: 1)
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Call.permissionDeniedBanner)
            }

            // ReplayKit Unavailable Banner (`simulateReplayKitDenied` or unavailable RPScreenRecorder)
            if controller.isScreenShareUnavailable {
                disclosureCard(
                    title: FabushiPlatformStrings.text(.callsScreenShareUnavailableTitle, locale: locale),
                    body: FabushiPlatformStrings.text(.callsScreenShareUnavailableBody, locale: locale),
                    systemImage: "rectangle.on.rectangle.slash",
                    tint: theme.textSecondary
                )
                .accessibilityIdentifier(FabushiAccessibilityIDs.Call.screenShareUnavailableBanner)
            }

            // Video Stage (LiveKitWebRTC Metal Video Renderer)
            if controller.isVideoCall || controller.localVideoTrack != nil || controller.remoteVideoTrack != nil {
                HumanCallVideoStage(
                    remoteTrack: controller.remoteVideoTrack,
                    localTrack: controller.localVideoTrack,
                    localMirrored: !controller.isScreenSharing,
                    mediaState: controller.callState.localizedTitle(locale: locale)
                )
            } else {
                // Voice Call Identity Header
                VStack(spacing: 10) {
                    FabushiAvatarBadge(
                        peerId: Int64(controller.activePeerUserId ?? 0),
                        title: controller.activePeerTitle.isEmpty ? "Fabushi" : controller.activePeerTitle,
                        isOnline: controller.callState == .connected,
                        isBot: false
                    )
                    Text(controller.activePeerTitle.isEmpty
                        ? FabushiPlatformStrings.text(.callsTabTitle, locale: locale)
                        : controller.activePeerTitle)
                        .fabushiFont(.title3)
                        .foregroundStyle(theme.textPrimary)

                    HStack(spacing: 8) {
                        Text(controller.callState.localizedTitle(locale: locale))
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textSecondary)
                        if controller.callState == .connected {
                            Text("· \(FabushiPlatformStrings.callDurationFormatted(controller.elapsedSeconds))")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.accentColor)
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
            }

            // 4-Emoji E2EE Fingerprint Verification Badge
            if let fingerprint = controller.emojiFingerprint, !fingerprint.isEmpty {
                HStack(spacing: 10) {
                    Image(systemName: "lock.shield.fill")
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.successColor)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(FabushiPlatformStrings.text(.callsFingerprintTitle, locale: locale))
                            .fabushiFont(.caption2)
                            .foregroundStyle(theme.textSecondary)
                        Text(fingerprint)
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.textPrimary)
                    }
                    Spacer(minLength: 4)
                }
                .padding(12)
                .background(
                    theme.surfaceSecondary,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Call.fingerprintBadge)
            }

            // Incoming Accept / Decline Action Bar
            if controller.callState == .ringingIncoming {
                HStack(spacing: 16) {
                    Button {
                        Task { await controller.declineIncomingCall(locale: locale) }
                    } label: {
                        Label(
                            FabushiPlatformStrings.text(.callsDeclineAction, locale: locale),
                            systemImage: "phone.down.fill"
                        )
                        .fabushiFont(.headline)
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                        .background(theme.destructiveColor, in: Capsule())
                        .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)

                    Button {
                        Task { await controller.acceptIncomingCall(locale: locale) }
                    } label: {
                        Label(
                            FabushiPlatformStrings.text(.callsAcceptAction, locale: locale),
                            systemImage: "phone.fill"
                        )
                        .fabushiFont(.headline)
                        .foregroundStyle(.white)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                        .background(theme.successColor, in: Capsule())
                        .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }

            // Primary Active Call Control Dock
            HStack(spacing: 12) {
                callActionButton(
                    title: controller.isMuted
                        ? FabushiPlatformStrings.text(.callsUnmuteAction, locale: locale)
                        : FabushiPlatformStrings.text(.callsMuteAction, locale: locale),
                    systemImage: controller.isMuted ? "mic.slash.fill" : "mic.fill",
                    isActive: controller.isMuted,
                    isDestructive: false,
                    accessibilityID: FabushiAccessibilityIDs.Call.muteButton
                ) {
                    controller.toggleMute(locale: locale)
                }

                callActionButton(
                    title: controller.isCameraEnabled
                        ? FabushiPlatformStrings.text(.callsCameraOffAction, locale: locale)
                        : FabushiPlatformStrings.text(.callsCameraOnAction, locale: locale),
                    systemImage: controller.isCameraEnabled ? "video.fill" : "video.slash.fill",
                    isActive: controller.isCameraEnabled,
                    isDestructive: false,
                    accessibilityID: FabushiAccessibilityIDs.Call.cameraButton
                ) {
                    Task { await controller.toggleCamera(locale: locale) }
                }

                callActionButton(
                    title: controller.isScreenSharing
                        ? FabushiPlatformStrings.text(.callsStopScreenShareAction, locale: locale)
                        : FabushiPlatformStrings.text(.callsScreenShareAction, locale: locale),
                    systemImage: "rectangle.inset.filled.and.person.filled",
                    isActive: controller.isScreenSharing,
                    isDestructive: false,
                    accessibilityID: FabushiAccessibilityIDs.Call.screenShareButton
                ) {
                    Task { await controller.toggleScreenShare(locale: locale) }
                }

                callActionButton(
                    title: FabushiPlatformStrings.text(.callsHangupAction, locale: locale),
                    systemImage: "phone.down.fill",
                    isActive: false,
                    isDestructive: true,
                    accessibilityID: FabushiAccessibilityIDs.Call.hangupButton
                ) {
                    Task { await controller.hangup(locale: locale) }
                }
            }
            .padding(12)
            .fabushiLiquidGlass(cornerRadius: 18, isInteractive: true)

            // Native Apple ReplayKit Broadcast Picker (`FabushiBroadcastUpload`)
            if !controller.isScreenShareUnavailable {
                HStack(spacing: 12) {
                    HumanCallBroadcastPicker()
                        .frame(width: 44, height: 44)
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Call.screenSharePicker)

                    Text(FabushiPlatformStrings.text(.callsScreenShareAction, locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)

                    Spacer(minLength: 8)

                    if controller.callState.isActive {
                        Button {
                            controller.minimizeToFloatingPip(locale: locale)
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.callsMinimizePipAction, locale: locale),
                                systemImage: "pip.enter"
                            )
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.accentColor)
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    theme.surfaceSecondary,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )
            }
        }
        .padding(16)
        .background(
            theme.surfacePrimary,
            in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Call.overlay)
    }

    private func callActionButton(
        title: String,
        systemImage: String,
        isActive: Bool,
        isDestructive: Bool,
        accessibilityID: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: systemImage)
                    .fabushiFont(.headline)
                    .foregroundStyle(
                        isDestructive ? .white : (isActive ? theme.accentForegroundColor : theme.textPrimary)
                    )
                    .frame(width: 48, height: 48)
                    .background(
                        isDestructive
                            ? theme.destructiveColor
                            : (isActive ? theme.accentColor : theme.surfaceElevated),
                        in: Circle()
                    )
                Text(title)
                    .fabushiFont(.caption2)
                    .foregroundStyle(theme.textSecondary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity)
            .fabushiMinimumTouchTarget()
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityIdentifier(accessibilityID)
    }

    private func disclosureCard(
        title: String,
        body: String,
        systemImage: String,
        tint: Color
    ) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: systemImage)
                .fabushiFont(.subheadline)
                .foregroundStyle(tint)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textPrimary)
                Text(body)
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
            Spacer(minLength: 4)
        }
        .padding(12)
        .background(
            theme.surfaceSecondary,
            in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
        )
        .accessibilityElement(children: .combine)
    }
}
