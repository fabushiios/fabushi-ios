import SwiftUI

/// Modal Calls & Full-Duplex AI Talk Mode Sheet returned by `PlatformModulesProviding.makeCallsTabView`.
///
/// Wraps in its OWN `NavigationStack` because `FabushiAdaptiveShellView` presents `.calls` as a modal `.sheet`,
/// providing an explicit Close toolbar action calling `router.dismissActiveModalSheet(coordinator:)`.
@MainActor
public struct FabushiCallsTabView: View {
    public enum Segment: String, CaseIterable, Sendable, Identifiable {
        case voiceVideo
        case talkMode

        public var id: String { rawValue }
    }

    @State private var callController: FabushiCallSessionController
    @State private var talkViewModel: FabushiTalkModeViewModel
    @State private var selectedSegment: Segment = .voiceVideo

    @State private var targetUserIdInput: String = ""
    @State private var groupChatIdInput: String = ""
    @State private var selectedRatingStars: UInt8 = 5
    @State private var ratingFeedbackDraft: String = ""

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration = .current
    ) {
        _callController = State(
            initialValue: FabushiCallSessionController(
                coordinator: coordinator,
                router: router,
                launchConfiguration: launchConfiguration
            )
        )
        _talkViewModel = State(
            initialValue: FabushiTalkModeViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Picker(
                    FabushiPlatformStrings.text(.callsTabTitle, locale: locale),
                    selection: $selectedSegment
                ) {
                    Text(FabushiPlatformStrings.text(.callsSegmentVoiceVideo, locale: locale))
                        .tag(Segment.voiceVideo)
                    Text(FabushiPlatformStrings.text(.callsSegmentTalkMode, locale: locale))
                        .tag(Segment.talkMode)
                }
                .pickerStyle(.segmented)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)

                switch selectedSegment {
                case .voiceVideo:
                    voiceAndVideoContent
                case .talkMode:
                    FabushiTalkModeOrbView(viewModel: talkViewModel)
                }
            }
            .background(theme.surfacePrimary)
            .navigationTitle(FabushiPlatformStrings.text(.callsTabTitle, locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        callController.router.dismissActiveModalSheet(coordinator: callController.coordinator)
                    } label: {
                        Text(FabushiPlatformStrings.text(.close, locale: locale))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.backButton)
                }

                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task { await callController.loadCallHistory() }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(FabushiPlatformStrings.text(.refresh, locale: locale))
                }
            }
            .sheet(isPresented: $callController.isRatingSheetPresented) {
                postCallRatingSheet
            }
            .task {
                callController.bindSystemCallCoordinator()
                callController.refreshCapabilities()
                await callController.loadCallHistory()
            }
            .onDisappear {
                if !callController.callState.isActive && callController.router.activeCallPip == nil {
                    callController.detachLifecycleHooks()
                }
            }
        }
    }

    private var voiceAndVideoContent: some View {
        ScrollView {
            VStack(spacing: 16) {
                // Active Call & Media Controls Overlay (always reachable for permissions, ReplayKit & BLK-02)
                FabushiCallOverlayView(controller: callController)

                // Outgoing 1:1 Call Launcher Card
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        FabushiSectionHeader(FabushiPlatformStrings.text(.callsSegmentVoiceVideo, locale: locale))
                        Spacer()
                        Label(
                            callController.voipTokenRegistered
                                ? FabushiPlatformStrings.text(.callsVoipRegisteredLabel, locale: locale)
                                : FabushiPlatformStrings.text(.callsVoipPendingLabel, locale: locale),
                            systemImage: "phone.badge.checkmark"
                        )
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textSecondary)
                    }

                    TextField(
                        FabushiPlatformStrings.text(.callsTargetUserIdPlaceholder, locale: locale),
                        text: $targetUserIdInput
                    )
                    .fabushiFont(.body)
                    .keyboardType(.numberPad)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(
                        theme.surfaceInput,
                        in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                    )

                    HStack(spacing: 10) {
                        Button {
                            let peerId = UInt64(targetUserIdInput.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 1001
                            Task {
                                _ = await callController.startOutgoingCall(
                                    calleeUserId: peerId,
                                    peerTitle: "User #\(peerId)",
                                    isVideo: false,
                                    locale: locale
                                )
                            }
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.callsStartVoiceCall, locale: locale),
                                systemImage: "phone.fill"
                            )
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.accentForegroundColor)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 10)
                            .background(theme.accentColor, in: Capsule())
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)

                        Button {
                            let peerId = UInt64(targetUserIdInput.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 1001
                            Task {
                                _ = await callController.startOutgoingCall(
                                    calleeUserId: peerId,
                                    peerTitle: "User #\(peerId)",
                                    isVideo: true,
                                    locale: locale
                                )
                            }
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.callsStartVideoCall, locale: locale),
                                systemImage: "video.fill"
                            )
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textPrimary)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 10)
                            .background(theme.surfaceElevated, in: Capsule())
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(14)
                .background(
                    theme.surfaceSecondary,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )

                // Group SFU Room Card (`calls.sfuJoinRoom` / `sfuLeaveRoom`)
                VStack(alignment: .leading, spacing: 10) {
                    TextField(
                        FabushiPlatformStrings.text(.callsGroupRoomChatIdPlaceholder, locale: locale),
                        text: $groupChatIdInput
                    )
                    .fabushiFont(.body)
                    .keyboardType(.numbersAndPunctuation)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(
                        theme.surfaceInput,
                        in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                    )

                    HStack(spacing: 10) {
                        Button {
                            let chatId = Int64(groupChatIdInput.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 100
                            Task { _ = await callController.joinGroupRoom(chatId: chatId) }
                        } label: {
                            Text(FabushiPlatformStrings.text(.callsJoinGroupRoom, locale: locale))
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.accentColor)
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)

                        if callController.activeGroupRoomChatId != nil {
                            Button {
                                Task { _ = await callController.leaveGroupRoom() }
                            } label: {
                                Text(FabushiPlatformStrings.text(.callsLeaveGroupRoom, locale: locale))
                                    .fabushiFont(.subheadline)
                                    .foregroundStyle(theme.destructiveColor)
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                        }

                        Spacer()
                        if !callController.groupRoomSubscribers.isEmpty {
                            Text("\(callController.groupRoomSubscribers.count)")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                }
                .padding(14)
                .background(
                    theme.surfaceSecondary,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                )

                // Call History List (`calls.list`)
                VStack(alignment: .leading, spacing: 10) {
                    FabushiSectionHeader(FabushiPlatformStrings.text(.callsHistoryTitle, locale: locale))
                    if callController.callHistory.isEmpty {
                        FabushiEmptyStateView(
                            title: FabushiPlatformStrings.text(.callsHistoryEmptyTitle, locale: locale),
                            systemImage: "phone.arrow.up.right",
                            descriptionText: FabushiPlatformStrings.text(.callsHistoryEmptySubtitle, locale: locale),
                            accessibilityID: "fabushi.call.historyEmpty"
                        )
                    } else {
                        ForEach(callController.callHistory) { record in
                            HStack(spacing: 12) {
                                Image(systemName: record.isVideo ? "video.fill" : "phone.fill")
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.accentColor)
                                    .accessibilityHidden(true)

                                VStack(alignment: .leading, spacing: 2) {
                                    Text("#\(record.calleeId) · \(record.state)")
                                        .fabushiFont(.subheadline)
                                        .foregroundStyle(theme.textPrimary)
                                    Text(FabushiPlatformStrings.callDurationFormatted(Int(record.durationSecs)))
                                        .fabushiFont(.monospacedCaption)
                                        .foregroundStyle(theme.textSecondary)
                                }

                                Spacer()

                                if let rating = record.rating {
                                    Label("\(rating)/5", systemImage: "star.fill")
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.warningColor)
                                } else {
                                    Button {
                                        callController.presentRatingSheet(for: record.callId)
                                    } label: {
                                        Text(FabushiPlatformStrings.text(.callsRateSubmitAction, locale: locale))
                                            .fabushiFont(.caption)
                                            .foregroundStyle(theme.accentColor)
                                            .fabushiMinimumTouchTarget()
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                            .padding(12)
                            .background(
                                theme.surfaceSecondary,
                                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                            )
                        }
                    }
                }
            }
            .padding(16)
        }
    }

    private var postCallRatingSheet: some View {
        NavigationStack {
            Form {
                Section(FabushiPlatformStrings.text(.callsRateCallTitle, locale: locale)) {
                    HStack(spacing: 10) {
                        ForEach(1...5, id: \.self) { star in
                            Button {
                                selectedRatingStars = UInt8(star)
                            } label: {
                                Image(systemName: UInt8(star) <= selectedRatingStars ? "star.fill" : "star")
                                    .fabushiFont(.title2)
                                    .foregroundStyle(theme.warningColor)
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("\(star)")
                        }
                    }

                    TextField(
                        FabushiPlatformStrings.text(.callsRateFeedbackPlaceholder, locale: locale),
                        text: $ratingFeedbackDraft
                    )
                    .fabushiFont(.body)
                }

                Section {
                    Button {
                        guard let callId = callController.pendingRatingCallId else {
                            callController.isRatingSheetPresented = false
                            return
                        }
                        Task {
                            _ = await callController.submitCallRating(
                                callId: callId,
                                rating: selectedRatingStars,
                                feedback: ratingFeedbackDraft,
                                locale: locale
                            )
                            ratingFeedbackDraft = ""
                        }
                    } label: {
                        Text(FabushiPlatformStrings.text(.callsRateSubmitAction, locale: locale))
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.accentColor)
                            .frame(maxWidth: .infinity)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }
            .navigationTitle(FabushiPlatformStrings.text(.callsRateCallTitle, locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.close, locale: locale)) {
                        callController.isRatingSheetPresented = false
                    }
                }
            }
        }
        .presentationDetents([.medium])
    }
}
