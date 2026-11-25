import SwiftUI
import UIKit

/// Standalone 4-Tab NetEase UU-Remote-style Computer Hub (`Feature #27` + `D10-11`).
///
/// NOTE: `FabushiAdaptiveShellView` already wraps `makeRemoteComputerWorkbenchView` inside a
/// `NavigationStack`, so this view intentionally does NOT nest a second `NavigationStack` at root.
/// It NEVER renders the macOS drag-permission HUD (forbidden on iOS).
@MainActor
public struct FabushiRemoteComputerHubView: View {
    public let initialNodeId: String?
    @State private var viewModel: FabushiRemoteComputerViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var assistTargetId: String = ""
    @State private var assistTargetCode: String = ""
    @State private var assistTargetMode: FabushiRemoteControlMode = .desktop
    @State private var customCodeDraft: String = ""
    @State private var isCustomCodeSheetPresented: Bool = false

    // Cloud PC Form State
    @State private var cloudName: String = ""
    @State private var cloudHost: String = ""
    @State private var cloudPort: String = "22"
    @State private var cloudUsername: String = "root"
    @State private var cloudAuthMethod: FabushiCloudAuthMethod = .sshPassword
    @State private var cloudSecret: String = ""
    @State private var cloudWorkdir: String = ""

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(
        nodeId: String?,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) {
        self.initialNodeId = nodeId
        _viewModel = State(
            initialValue: FabushiRemoteComputerViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        Group {
            if let node = viewModel.activeWorkbenchNode {
                FabushiRemoteWorkbenchView(node: node, viewModel: viewModel)
            } else {
                hubOverviewLayout
            }
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.remoteHubTitle, locale: locale))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.workbench)
        .task(id: viewModel.coordinator.accountScopeGeneration) {
            viewModel.reconcileAccountScopeIfNeeded()
            await viewModel.refreshAll()
            viewModel.openNodeIfPresent(initialNodeId)
        }
        .onDisappear {
            viewModel.detachLifecycleHooks()
        }
        .sheet(isPresented: $viewModel.isPermissionModalPresented) {
            capabilityDiagnosticsModal
        }
        .sheet(isPresented: $viewModel.isCloudPCFormPresented) {
            cloudPCFormSheet
        }
        .sheet(isPresented: $isCustomCodeSheetPresented) {
            customAssistCodeSheet
        }
        .sheet(isPresented: $viewModel.isBrowserSheetPresented) {
            NavigationStack {
                FabushiBrowserWorkspaceView(
                    initialURL: nil,
                    coordinator: viewModel.coordinator,
                    router: viewModel.router
                )
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(FabushiPlatformStrings.text(.close, locale: locale)) {
                            viewModel.isBrowserSheetPresented = false
                        }
                        .fabushiMinimumTouchTarget()
                    }
                }
            }
        }
    }

    // MARK: - Adaptive Hub Overview (iPad 2-Column Sidebar + Grid / iPhone 4-Tab Segmented)

    @ViewBuilder
    private var hubOverviewLayout: some View {
        if horizontalSizeClass == .regular {
            HStack(spacing: 0) {
                iPadHubSidebar
                    .frame(width: FabushiLayoutMetrics.sidebarMinWidth)
                    .background(theme.surfaceSecondary)
                Divider()
                hubTabContent
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        } else {
            VStack(spacing: 0) {
                inlineControlStrip
                Picker(
                    FabushiPlatformStrings.text(.remoteHubTitle, locale: locale),
                    selection: $viewModel.selectedHubTab
                ) {
                    ForEach(FabushiRemoteHubTab.allCases) { tab in
                        Text(tab.displayTitle(locale: locale)).tag(tab)
                    }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal, 16)
                .padding(.bottom, 8)
                Divider()
                hubTabContent
            }
        }
    }

    private var iPadHubSidebar: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                ForEach(FabushiRemoteHubTab.allCases) { tab in
                    let isSelected = viewModel.selectedHubTab == tab
                    Button {
                        viewModel.selectedHubTab = tab
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: tab.systemImage)
                                .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.accentColor)
                            Text(tab.displayTitle(locale: locale))
                                .fabushiFont(.subheadline)
                                .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textPrimary)
                            Spacer()
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(
                            isSelected ? theme.accentColor : Color.clear,
                            in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                        )
                    }
                    .fabushiMinimumTouchTarget()
                }

                Divider()

                inlineControlStripContents
            }
            .padding(14)
        }
    }

    private var inlineControlStrip: some View {
        VStack(spacing: 10) {
            inlineControlStripContents
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(theme.surfaceTopBar)
    }

    private var inlineControlStripContents: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Live Tailnet Status Badge (IP · relay · RTT)
            HStack(spacing: 8) {
                Circle()
                    .fill(viewModel.tailnetStatus.running ? theme.successColor : theme.textMuted)
                    .frame(width: 8, height: 8)
                Text(
                    FabushiPlatformStrings.tailnetSummary(
                        ip: viewModel.tailnetStatus.tailnetIp,
                        relayLabel: viewModel.tailnetStatus.relayMode.displayTitle(locale: locale),
                        rttMs: viewModel.tailnetStatus.derpRttMs,
                        locale: locale
                    )
                )
                .fabushiFont(.caption)
                .foregroundStyle(theme.textPrimary)
                .lineLimit(1)
                Spacer()
                Button {
                    Task {
                        if viewModel.tailnetStatus.running {
                            await viewModel.stopTailnet()
                        } else {
                            await viewModel.startTailnet()
                        }
                    }
                } label: {
                    Text(
                        viewModel.tailnetStatus.running
                            ? FabushiPlatformStrings.text(.remoteTailnetStop, locale: locale)
                            : FabushiPlatformStrings.text(.remoteTailnetStart, locale: locale)
                    )
                    .fabushiFont(.caption)
                }
                .fabushiMinimumTouchTarget()
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.tailnetStatusBadge)

            // Live Allow Remote Optimistic Toggle + Inline Disconnect All (N)
            HStack(spacing: 10) {
                Toggle(
                    FabushiPlatformStrings.text(.remoteAllowToggle, locale: locale),
                    isOn: Binding(
                        get: { viewModel.assistConfig.allowAssist },
                        set: { newValue in
                            Task {
                                _ = await viewModel.toggleAllowRemote(newValue, locale: locale)
                            }
                        }
                    )
                )
                .fabushiFont(.subheadline)
                .disabled(viewModel.isUpdatingAllowRemote)
            }

            HStack(spacing: 8) {
                Button {
                    Task {
                        _ = await viewModel.disconnectAllSessions(locale: locale)
                    }
                } label: {
                    Label(
                        FabushiPlatformStrings.disconnectAllTitle(
                            count: viewModel.activeSessionCount,
                            locale: locale
                        ),
                        systemImage: "xmark.shield.fill"
                    )
                    .fabushiFont(.caption)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .padding(.horizontal, 10)
                    .background(theme.destructiveColor.opacity(0.14), in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .foregroundStyle(theme.destructiveColor)
                }
                .fabushiMinimumTouchTarget()
                .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.emergencyTakeoverButton)

                Button {
                    viewModel.isPermissionModalPresented = true
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.remoteCapabilityDiagnostics, locale: locale),
                        systemImage: "info.circle"
                    )
                    .fabushiFont(.caption)
                    .padding(.vertical, 8)
                    .padding(.horizontal, 10)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .foregroundStyle(theme.textPrimary)
                }
                .fabushiMinimumTouchTarget()
            }
        }
    }

    // MARK: - 4-Tab Content

    private var hubTabContent: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                // Honest iOS Sandbox Banner (Always Visible — ZERO macOS Drag HUD)
                iosSandboxBanner

                if !viewModel.activeSessions.isEmpty || viewModel.router.activeRemoteControlStatus != nil {
                    activeControlledBanner
                }

                switch viewModel.selectedHubTab {
                case .myDevices:
                    myDevicesTab
                case .remoteAssist:
                    remoteAssistTab
                case .cloudPCs:
                    cloudPCsTab
                case .aiAndMcp:
                    aiAndMcpTab
                }

                // D16-01: Built-in Interactive Browser & Automation App Store Entry Card
                builtInBrowserLauncherCard
            }
            .padding(16)
        }
        .refreshable {
            await viewModel.refreshAll()
        }
    }

    private var iosSandboxBanner: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "iphone.radiowaves.left.and.right")
                .fabushiFont(.headline)
                .foregroundStyle(theme.accentColor)
            VStack(alignment: .leading, spacing: 4) {
                Text(FabushiPlatformStrings.text(.remoteLocalHostUnsupportedTitle, locale: locale))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textPrimary)
                Text(FabushiPlatformStrings.text(.remoteLocalHostUnsupportedBody, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
        )
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.localHostUnsupportedBanner)
    }

    private var activeControlledBanner: some View {
        HStack(spacing: 10) {
            Image(systemName: "bolt.shield.fill")
                .foregroundStyle(theme.warningColor)
            VStack(alignment: .leading, spacing: 2) {
                Text(FabushiPlatformStrings.text(.remoteControlledBannerTitle, locale: locale))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textPrimary)
                Text(FabushiPlatformStrings.activeSessionsSummary(count: max(1, viewModel.activeSessionCount), locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
            Spacer()
            Button {
                Task {
                    _ = await viewModel.disconnectAllSessions(locale: locale)
                }
            } label: {
                Text(FabushiPlatformStrings.text(.remoteEmergencyTakeover, locale: locale))
                    .fabushiFont(.caption)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.destructiveColor, in: Capsule())
                    .foregroundStyle(Color.white)
            }
            .fabushiMinimumTouchTarget()
        }
        .padding(12)
        .background(theme.warningColor.opacity(0.14), in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.controlledBanner)
    }

    // MARK: - Tab 1: My Devices (Visual Device Cards + Quick Assist Strip + Add Cloud PC Ghost Card)

    private var myDevicesTab: some View {
        VStack(alignment: .leading, spacing: 16) {
            quickAssistStrip

            VStack(alignment: .leading, spacing: 12) {
                let nodes = viewModel.computers
                if nodes.isEmpty {
                    FabushiEmptyStateView(
                        title: FabushiPlatformStrings.text(.remoteEmptyDevicesTitle, locale: locale),
                        systemImage: "desktopcomputer",
                        descriptionText: FabushiPlatformStrings.text(.remoteEmptyDevicesSubtitle, locale: locale),
                        accessibilityID: "fabushi.remote.emptyState"
                    )
                } else {
                    let columns = horizontalSizeClass == .regular
                        ? [GridItem(.adaptive(minimum: 280), spacing: 14)]
                        : [GridItem(.flexible())]
                    LazyVGrid(columns: columns, spacing: 14) {
                        ForEach(Array(nodes.enumerated()), id: \.element.id) { index, node in
                            deviceCard(node: node, index: index)
                        }
                        addCloudPCGhostCard
                    }
                }
                if nodes.isEmpty {
                    addCloudPCGhostCard
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.nodeList)
        }
    }

    private func deviceCard(node: FabushiComputerNodeDto, index: Int) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            // Visual Thumbnail Preview (`latest_frame_data_url` or Stylized OS Wallpaper Fallback)
            ZStack(alignment: .topLeading) {
                if let image = FabushiRemoteImageDecoder.decodeDataURL(
                    viewModel.latestFrameByNodeId[node.nodeId] ?? node.latestFrameDataUrl
                ) {
                    Image(uiImage: image)
                        .resizable()
                        .aspectRatio(contentMode: .fill)
                        .frame(height: 132)
                        .clipped()
                } else {
                    LinearGradient(
                        colors: node.isCloudNode
                            ? [theme.accentColor.opacity(0.35), theme.surfaceElevated]
                            : [theme.accentColor.opacity(0.22), theme.surfaceSecondary],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                    .frame(height: 132)
                    .overlay {
                        VStack(spacing: 6) {
                            Image(systemName: node.osSymbolName)
                                .fabushiFont(.title2)
                                .foregroundStyle(theme.accentColor)
                            Text(node.osName.isEmpty ? (node.isCloudNode ? "Cloud Linux" : "Desktop") : node.osName)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                }

                // Metadata Pills Overlay
                HStack(spacing: 6) {
                    statusPill(
                        text: node.online
                            ? FabushiPlatformStrings.text(.online, locale: locale)
                            : FabushiPlatformStrings.text(.offline, locale: locale),
                        tint: node.online ? theme.successColor : theme.textMuted
                    )
                    if node.isCurrentDevice {
                        statusPill(
                            text: FabushiPlatformStrings.text(.remoteThisPCBadge, locale: locale),
                            tint: theme.accentColor
                        )
                    } else if node.isCloudNode {
                        statusPill(
                            text: FabushiPlatformStrings.text(.remoteCloudBadge, locale: locale),
                            tint: theme.accentColor
                        )
                    }
                }
                .padding(8)
            }
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            VStack(alignment: .leading, spacing: 4) {
                Text(node.displayName)
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(1)

                let metaParts = [
                    node.ipAddress.isEmpty ? (node.host ?? "") : node.ipAddress,
                    node.resolution,
                    node.runPolicy.displayTitle(locale: locale),
                ].filter { !$0.isEmpty }
                Text(metaParts.joined(separator: " · "))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .lineLimit(1)
            }

            // 3 Primary Actions: Enter Desktop, Shell, Settings
            HStack(spacing: 8) {
                Button {
                    viewModel.openDesktop(node: node)
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.remoteActionEnterDesktop, locale: locale),
                        systemImage: "display"
                    )
                    .fabushiFont(.caption)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 8)
                    .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .foregroundStyle(theme.accentForegroundColor)
                }
                .fabushiMinimumTouchTarget()

                Button {
                    viewModel.openShell(node: node)
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.remoteActionShell, locale: locale),
                        systemImage: "terminal"
                    )
                    .fabushiFont(.caption)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                    .foregroundStyle(theme.textPrimary)
                }
                .fabushiMinimumTouchTarget()

                Button {
                    viewModel.openDeviceSettings(node: node)
                } label: {
                    Image(systemName: "gearshape")
                        .fabushiFont(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 8)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                        .foregroundStyle(theme.textPrimary)
                }
                .accessibilityLabel(FabushiPlatformStrings.text(.remoteActionSettings, locale: locale))
                .fabushiMinimumTouchTarget()
            }
        }
        .padding(12)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(index == 0 ? FabushiAccessibilityIDs.Remote.firstNodeRow : "fabushi.remote.node.\(index)")
    }

    private func statusPill(text: String, tint: Color) -> some View {
        Text(text)
            .fabushiFont(.caption2)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(tint.opacity(0.22), in: Capsule())
            .foregroundStyle(tint)
    }

    private var addCloudPCGhostCard: some View {
        Button {
            viewModel.isCloudPCFormPresented = true
        } label: {
            VStack(spacing: 8) {
                Image(systemName: "plus.circle.dashed")
                    .fabushiFont(.title2)
                    .foregroundStyle(theme.accentColor)
                Text(FabushiPlatformStrings.text(.remoteAddCloudPC, locale: locale))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                Text(FabushiPlatformStrings.text(.remoteAddCloudPCSubtitle, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .multilineTextAlignment(.center)
            }
            .padding(20)
            .frame(maxWidth: .infinity, minHeight: 132)
            .overlay(
                RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                    .strokeBorder(
                        theme.accentColor.opacity(0.55),
                        style: StrokeStyle(lineWidth: 1.25, dash: [6, 4])
                    )
            )
        }
        .fabushiMinimumTouchTarget()
    }

    // MARK: - Quick Assist Strip (`D10-11`)

    private var quickAssistStrip: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(
                    FabushiPlatformStrings.text(.remoteQuickAssistTitle, locale: locale),
                    systemImage: "person.badge.key.fill"
                )
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
                Spacer()
                Button {
                    viewModel.selectedHubTab = .remoteAssist
                } label: {
                    Text(FabushiPlatformStrings.text(.remoteAssistOthersAction, locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.accentColor)
                }
                .fabushiMinimumTouchTarget()
            }

            HStack(spacing: 16) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(FabushiPlatformStrings.text(.remoteAssistIdLabel, locale: locale))
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textSecondary)
                    Text(viewModel.assistConfig.assistId.isEmpty ? "— — —" : viewModel.assistConfig.formattedAssistId)
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(FabushiPlatformStrings.text(.remoteAccessCodeLabel, locale: locale))
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textSecondary)
                    Text(viewModel.assistConfig.currentCode ?? "••••••")
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                }
                Spacer()
            }

            HStack(spacing: 8) {
                Button {
                    Task { await viewModel.resetAssistCode(locale: locale) }
                } label: {
                    Text(FabushiPlatformStrings.text(.remoteResetCode, locale: locale))
                        .fabushiFont(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(theme.surfaceElevated, in: Capsule())
                }
                .fabushiMinimumTouchTarget()

                Button {
                    customCodeDraft = viewModel.assistConfig.currentCode ?? ""
                    isCustomCodeSheetPresented = true
                } label: {
                    Text(FabushiPlatformStrings.text(.remoteSetCustomCode, locale: locale))
                        .fabushiFont(.caption)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(theme.surfaceElevated, in: Capsule())
                }
                .fabushiMinimumTouchTarget()

                Button {
                    viewModel.copyAssistInvite(locale: locale)
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.remoteCopyInvite, locale: locale),
                        systemImage: "doc.on.doc"
                    )
                    .fabushiFont(.caption)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(theme.accentColor.opacity(0.16), in: Capsule())
                    .foregroundStyle(theme.accentColor)
                }
                .fabushiMinimumTouchTarget()
            }
        }
        .padding(14)
        .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
    }

    // MARK: - Tab 2: Remote Assist

    private var remoteAssistTab: some View {
        VStack(alignment: .leading, spacing: 14) {
            quickAssistStrip

            VStack(alignment: .leading, spacing: 12) {
                Text(FabushiPlatformStrings.text(.remoteAssistOthersAction, locale: locale))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)

                TextField(
                    FabushiPlatformStrings.text(.remoteAssistTargetIdPlaceholder, locale: locale),
                    text: $assistTargetId
                )
                .fabushiFont(.body)
                .keyboardType(.numberPad)
                .padding(12)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))

                TextField(
                    FabushiPlatformStrings.text(.remoteAssistAccessCodePlaceholder, locale: locale),
                    text: $assistTargetCode
                )
                .fabushiFont(.body)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .padding(12)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))

                Picker("", selection: $assistTargetMode) {
                    ForEach(FabushiRemoteControlMode.allCases) { mode in
                        Text(mode.displayTitle(locale: locale)).tag(mode)
                    }
                }
                .pickerStyle(.segmented)

                Button {
                    Task {
                        _ = await viewModel.startRemoteAssist(
                            assistId: assistTargetId,
                            accessCode: assistTargetCode,
                            mode: assistTargetMode,
                            locale: locale
                        )
                    }
                } label: {
                    Text(FabushiPlatformStrings.text(.remoteAssistConnectAction, locale: locale))
                        .fabushiFont(.headline)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                        .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                        .foregroundStyle(theme.accentForegroundColor)
                }
                .disabled(assistTargetId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || assistTargetCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .fabushiMinimumTouchTarget()
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }

    // MARK: - Tab 3: Cloud PCs

    private var cloudPCsTab: some View {
        VStack(alignment: .leading, spacing: 14) {
            ForEach(Array(viewModel.cloudComputers.enumerated()), id: \.element.id) { index, node in
                deviceCard(node: node, index: index)
            }
            addCloudPCGhostCard
        }
    }

    // MARK: - Tab 4: AI & MCP Approvals + Tailnet Relay Control

    private var aiAndMcpTab: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 10) {
                Text(FabushiPlatformStrings.text(.remoteTailnetTitle, locale: locale))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)

                Toggle(
                    FabushiPlatformStrings.text(.remoteTailnetForceDerp, locale: locale),
                    isOn: Binding(
                        get: { viewModel.forceDerpOnly },
                        set: { value in
                            Task { await viewModel.setForceDerpOnly(value) }
                        }
                    )
                )
                .fabushiFont(.subheadline)
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))

            VStack(alignment: .leading, spacing: 10) {
                Text(FabushiPlatformStrings.text(.remotePendingApprovalsTitle, locale: locale))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)

                if viewModel.pendingApprovals.isEmpty {
                    Text(FabushiPlatformStrings.text(.remoteNoPendingApprovals, locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(viewModel.pendingApprovals) { item in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(item.callerLabel)
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textPrimary)
                            Text(item.operation.summaryLabel)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                            HStack(spacing: 10) {
                                Button {
                                    Task { await viewModel.approveCommand(commandId: item.commandId, approved: true, locale: locale) }
                                } label: {
                                    Text(FabushiPlatformStrings.text(.remoteApprove, locale: locale))
                                        .fabushiFont(.caption)
                                        .padding(.horizontal, 12)
                                        .padding(.vertical, 8)
                                        .background(theme.successColor, in: Capsule())
                                        .foregroundStyle(Color.white)
                                }
                                .fabushiMinimumTouchTarget()

                                Button {
                                    Task { await viewModel.approveCommand(commandId: item.commandId, approved: false, locale: locale) }
                                } label: {
                                    Text(FabushiPlatformStrings.text(.remoteDeny, locale: locale))
                                        .fabushiFont(.caption)
                                        .padding(.horizontal, 12)
                                        .padding(.vertical, 8)
                                        .background(theme.destructiveColor, in: Capsule())
                                        .foregroundStyle(Color.white)
                                }
                                .fabushiMinimumTouchTarget()
                            }
                        }
                        .padding(12)
                        .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
        }
    }

    // MARK: - D16-01 Built-in Browser & Apps Launcher Card

    private var builtInBrowserLauncherCard: some View {
        Button {
            viewModel.isBrowserSheetPresented = true
        } label: {
            HStack(spacing: 12) {
                Image(systemName: "safari.fill")
                    .fabushiFont(.title2)
                    .foregroundStyle(theme.accentColor)
                VStack(alignment: .leading, spacing: 3) {
                    Text(FabushiPlatformStrings.text(.builtInBrowserTitle, locale: locale))
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                    Text(FabushiPlatformStrings.text(.builtInBrowserSubtitle, locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                        .multilineTextAlignment(.leading)
                }
                Spacer()
                Image(systemName: "chevron.right")
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textMuted)
            }
            .padding(14)
            .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                    .strokeBorder(theme.hairlineBorder, lineWidth: 0.75)
            )
        }
        .fabushiMinimumTouchTarget()
    }

    // MARK: - iOS Capability Diagnostic Modal (Honest Sandbox Statuses)

    private var capabilityDiagnosticsModal: some View {
        NavigationStack {
            List {
                Section {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(FabushiPlatformStrings.text(.remotePermissionScreenStatusTitle, locale: locale))
                            .fabushiFont(.headline)
                        Text(FabushiPlatformStrings.text(.remotePermissionScreenStatusValue, locale: locale))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.permissionModalScreenStatus)

                    VStack(alignment: .leading, spacing: 6) {
                        Text(FabushiPlatformStrings.text(.remotePermissionAccessibilityTitle, locale: locale))
                            .fabushiFont(.headline)
                        Text(FabushiPlatformStrings.text(.remotePermissionAccessibilityValue, locale: locale))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.permissionModalAccessibilityStatus)
                }

                Section {
                    Button {
                        Task { await viewModel.refreshTailnetStatus() }
                    } label: {
                        Text(FabushiPlatformStrings.text(.remotePermissionRetry, locale: locale))
                            .fabushiFont(.headline)
                    }
                    .fabushiMinimumTouchTarget()
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.permissionModalRetryButton)

                    Button {
                        _ = viewModel.unwindBackNavigation()
                    } label: {
                        Text(FabushiPlatformStrings.text(.remotePermissionDismiss, locale: locale))
                            .fabushiFont(.body)
                    }
                    .fabushiMinimumTouchTarget()
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.permissionModalDismissButton)
                }
            }
            .navigationTitle(FabushiPlatformStrings.text(.remotePermissionModalTitle, locale: locale))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.permissionModal)
        }
    }

    // MARK: - Cloud PC Form Sheet

    private var cloudPCFormSheet: some View {
        NavigationStack {
            Form {
                TextField(FabushiPlatformStrings.text(.remoteCloudNamePlaceholder, locale: locale), text: $cloudName)
                TextField(FabushiPlatformStrings.text(.remoteCloudHostPlaceholder, locale: locale), text: $cloudHost)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                TextField(FabushiPlatformStrings.text(.remoteCloudPortPlaceholder, locale: locale), text: $cloudPort)
                    .keyboardType(.numberPad)
                TextField(FabushiPlatformStrings.text(.remoteCloudUsernamePlaceholder, locale: locale), text: $cloudUsername)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                Picker("Auth", selection: $cloudAuthMethod) {
                    ForEach(FabushiCloudAuthMethod.allCases) { method in
                        Text(method.displayTitle(locale: locale)).tag(method)
                    }
                }
                SecureField(FabushiPlatformStrings.text(.remoteCloudSecretPlaceholder, locale: locale), text: $cloudSecret)
                TextField(FabushiPlatformStrings.text(.remoteCloudWorkdirPlaceholder, locale: locale), text: $cloudWorkdir)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
            }
            .navigationTitle(FabushiPlatformStrings.text(.remoteCloudFormTitle, locale: locale))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.cancel, locale: locale)) {
                        viewModel.isCloudPCFormPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(FabushiPlatformStrings.text(.save, locale: locale)) {
                        let parsedPort = UInt16(cloudPort.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 22
                        let request = FabushiUpsertCloudComputerRequest(
                            displayName: cloudName.trimmingCharacters(in: .whitespacesAndNewlines),
                            host: cloudHost.trimmingCharacters(in: .whitespacesAndNewlines),
                            port: parsedPort,
                            username: cloudUsername.trimmingCharacters(in: .whitespacesAndNewlines),
                            authMethod: cloudAuthMethod,
                            secret: cloudSecret.isEmpty ? nil : cloudSecret,
                            workdir: cloudWorkdir.isEmpty ? nil : cloudWorkdir,
                            verifyConnection: true
                        )
                        Task {
                            _ = await viewModel.upsertCloudComputer(request, locale: locale)
                        }
                    }
                    .disabled(cloudName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || cloudHost.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }

    private var customAssistCodeSheet: some View {
        NavigationStack {
            Form {
                TextField(
                    FabushiPlatformStrings.text(.remoteAccessCodeLabel, locale: locale),
                    text: $customCodeDraft
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
            }
            .navigationTitle(FabushiPlatformStrings.text(.remoteSetCustomCode, locale: locale))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.cancel, locale: locale)) {
                        isCustomCodeSheetPresented = false
                    }
                    .fabushiMinimumTouchTarget()
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(FabushiPlatformStrings.text(.save, locale: locale)) {
                        let code = customCodeDraft
                        isCustomCodeSheetPresented = false
                        Task { await viewModel.setCustomAssistCode(code, locale: locale) }
                    }
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
