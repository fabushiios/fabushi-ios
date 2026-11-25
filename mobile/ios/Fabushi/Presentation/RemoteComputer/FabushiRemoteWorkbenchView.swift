import SwiftUI
import UIKit

/// Interactive Remote Computer Workbench featuring Desktop Stream (`streamCanvas`),
/// Multi-Session PTY (`ptyTerminal` / `ptyInputField`), Remote Files (`list_dir`),
/// and per-device Settings (`updatePolicy`, `testConnection`, Wake-on-LAN, Delete).
@MainActor
public struct FabushiRemoteWorkbenchView: View {
    public let node: FabushiComputerNodeDto
    @Bindable public var viewModel: FabushiRemoteComputerViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @FocusState private var isPtyInputFocused: Bool

    @State private var selectedPolicy: FabushiComputerRunPolicy
    @State private var wolMacAddress: String = ""
    @State private var wolBroadcastIP: String = ""
    @State private var remotePathInput: String = "."

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(node: FabushiComputerNodeDto, viewModel: FabushiRemoteComputerViewModel) {
        self.node = node
        self.viewModel = viewModel
        _selectedPolicy = State(initialValue: node.runPolicy)
    }

    public var body: some View {
        VStack(spacing: 0) {
            workbenchHeader
            Picker(
                FabushiPlatformStrings.text(.remoteHubTitle, locale: locale),
                selection: $viewModel.selectedWorkbenchTab
            ) {
                ForEach(FabushiRemoteWorkbenchTab.allCases) { tab in
                    Label(tab.displayTitle(locale: locale), systemImage: tab.systemImage)
                        .tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)

            Divider()

            Group {
                switch viewModel.selectedWorkbenchTab {
                case .desktop:
                    desktopStreamTab
                case .terminal:
                    ptyTerminalTab
                case .files:
                    remoteFilesTab
                case .settings:
                    deviceSettingsTab
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .background(theme.surfacePrimary)
        .onAppear {
            selectedPolicy = node.runPolicy
            if viewModel.selectedWorkbenchTab == .terminal && viewModel.shouldAutoFocusPtyInput {
                isPtyInputFocused = true
                viewModel.consumeAutoFocusPtyRequest()
            }
        }
        .onChange(of: viewModel.shouldAutoFocusPtyInput) { _, shouldFocus in
            if shouldFocus && viewModel.selectedWorkbenchTab == .terminal {
                isPtyInputFocused = true
                viewModel.consumeAutoFocusPtyRequest()
            }
        }
    }

    private var workbenchHeader: some View {
        HStack(spacing: 12) {
            Button {
                _ = viewModel.unwindBackNavigation()
            } label: {
                Label(
                    FabushiPlatformStrings.text(.back, locale: locale),
                    systemImage: "chevron.left"
                )
                .fabushiFont(.subheadline)
            }
            .fabushiMinimumTouchTarget()

            VStack(alignment: .leading, spacing: 2) {
                Text(node.displayName)
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
                    .lineLimit(1)
                Text("\(node.osName.isEmpty ? "Remote" : node.osName) · \(node.ipAddress.isEmpty ? (node.host ?? "") : node.ipAddress)")
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            Button {
                Task {
                    await viewModel.emergencyTakeover(locale: locale)
                }
            } label: {
                Label(
                    FabushiPlatformStrings.text(.remoteEmergencyTakeover, locale: locale),
                    systemImage: "hand.raised.fill"
                )
                .fabushiFont(.caption)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(theme.destructiveColor.opacity(0.16), in: Capsule())
                .foregroundStyle(theme.destructiveColor)
            }
            .fabushiMinimumTouchTarget()
            .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.emergencyTakeoverButton)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(theme.surfaceTopBar)
    }

    // MARK: - Desktop Stream Tab

    private var desktopStreamTab: some View {
        ScrollView {
            VStack(spacing: 14) {
                if !viewModel.activeSessions.isEmpty || viewModel.router.activeRemoteControlStatus != nil {
                    HStack(spacing: 10) {
                        Image(systemName: "antenna.radiowaves.left.and.right")
                            .foregroundStyle(theme.warningColor)
                        Text(FabushiPlatformStrings.text(.remoteControlledBannerTitle, locale: locale))
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textPrimary)
                        Spacer()
                        Text(FabushiPlatformStrings.activeSessionsSummary(count: max(1, viewModel.activeSessionCount), locale: locale))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    .padding(12)
                    .background(theme.warningColor.opacity(0.14), in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.controlledBanner)
                }

                // Interactive Stream Canvas with Touch / Trackpad Click & Drag
                GeometryReader { geo in
                    ZStack {
                        RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                            .fill(theme.surfaceElevated)
                            .overlay(
                                RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
                                    .strokeBorder(theme.hairlineBorder, lineWidth: 1)
                            )

                        if let image = FabushiRemoteImageDecoder.decodeDataURL(
                            viewModel.latestFrameByNodeId[node.nodeId] ?? node.latestFrameDataUrl
                        ) {
                            Image(uiImage: image)
                                .resizable()
                                .aspectRatio(contentMode: .fit)
                                .clipShape(RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
                        } else {
                            VStack(spacing: 10) {
                                Image(systemName: node.osSymbolName)
                                    .fabushiFont(.largeTitle)
                                    .foregroundStyle(theme.accentColor)
                                Text(node.displayName)
                                    .fabushiFont(.headline)
                                    .foregroundStyle(theme.textPrimary)
                                Text(node.resolution.isEmpty ? "1920×1080" : node.resolution)
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                            }
                            .padding()
                        }
                    }
                    .contentShape(Rectangle())
                    .gesture(
                        SpatialTapGesture()
                            .onEnded { value in
                                let width = max(1, geo.size.width)
                                let height = max(1, geo.size.height)
                                let normX = Int32((value.location.x / width) * 1920.0)
                                let normY = Int32((value.location.y / height) * 1080.0)
                                Task {
                                    await viewModel.sendPointerAction(
                                        nodeId: node.nodeId,
                                        action: "click",
                                        x: normX,
                                        y: normY
                                    )
                                }
                            }
                    )
                }
                .frame(minHeight: 240, maxHeight: 360)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.streamCanvas)
                .accessibilityLabel(node.displayName)

                // Remote Desktop Control Toolbar
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 132), spacing: 10)], spacing: 10) {
                    toolbarActionButton(
                        title: FabushiPlatformStrings.text(.remoteWorkbenchCaptureFrame, locale: locale),
                        systemImage: "camera.viewfinder"
                    ) {
                        await viewModel.captureDesktopFrame(nodeId: node.nodeId)
                    }
                    toolbarActionButton(
                        title: FabushiPlatformStrings.text(.remoteWorkbenchDisplaySwitch, locale: locale),
                        systemImage: "rectangle.on.rectangle"
                    ) {
                        let nextDisplay: UInt32 = viewModel.activeDisplayIndex == 0 ? 1 : 0
                        await viewModel.switchDisplay(nodeId: node.nodeId, displayIndex: nextDisplay)
                    }
                    toolbarActionButton(
                        title: FabushiPlatformStrings.text(.remoteWorkbenchPrivacyScreen, locale: locale),
                        systemImage: viewModel.privacyScreenEnabled ? "eye.slash.fill" : "eye.fill"
                    ) {
                        await viewModel.togglePrivacyScreen(
                            nodeId: node.nodeId,
                            enabled: !viewModel.privacyScreenEnabled
                        )
                    }
                    toolbarActionButton(
                        title: FabushiPlatformStrings.text(.remoteWorkbenchClipboardPull, locale: locale),
                        systemImage: "doc.on.clipboard"
                    ) {
                        await viewModel.pullRemoteClipboard(nodeId: node.nodeId, locale: locale)
                    }
                    toolbarActionButton(
                        title: FabushiPlatformStrings.text(.remoteWorkbenchClipboardPush, locale: locale),
                        systemImage: "arrow.up.doc"
                    ) {
                        await viewModel.pushLocalClipboard(nodeId: node.nodeId, locale: locale)
                    }
                    toolbarActionButton(
                        title: FabushiPlatformStrings.text(.remoteWorkbenchPowerLock, locale: locale),
                        systemImage: "lock.fill"
                    ) {
                        await viewModel.sendPowerAction(nodeId: node.nodeId, kind: .lock)
                    }
                }

                // Hardware Keyboard / Hotkey Bar (uses remote.remapModifiers)
                VStack(alignment: .leading, spacing: 8) {
                    Text(FabushiPlatformStrings.text(.remoteWorkbenchSendKey, locale: locale))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)

                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            hotkeyChip("Copy (⌘C)", key: "c", modifiers: ["cmd"])
                            hotkeyChip("Paste (⌘V)", key: "v", modifiers: ["cmd"])
                            hotkeyChip("Switch App (⌘⇥)", key: "Tab", modifiers: ["cmd"])
                            hotkeyChip("Esc", key: "Escape", modifiers: [])
                            hotkeyChip("Ctrl+C", key: "c", modifiers: ["ctrl"])
                        }
                    }
                }
                .padding(12)
                .background(theme.surfaceSecondary, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous))
            }
            .padding(16)
        }
    }

    private func toolbarActionButton(
        title: String,
        systemImage: String,
        action: @escaping @MainActor () async -> Void
    ) -> some View {
        Button {
            Task { await action() }
        } label: {
            Label(title, systemImage: systemImage)
                .fabushiFont(.caption)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
                .padding(.horizontal, 10)
                .background(theme.surfaceElevated, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                .foregroundStyle(theme.textPrimary)
        }
        .fabushiMinimumTouchTarget()
    }

    private func hotkeyChip(_ title: String, key: String, modifiers: [String]) -> some View {
        Button {
            let hostOS = node.osName.lowercased().contains("win") ? "windows" : "macos"
            Task {
                await viewModel.remapAndSendHotkey(
                    nodeId: node.nodeId,
                    key: key,
                    modifiers: modifiers,
                    hostOS: hostOS
                )
            }
        } label: {
            Text(title)
                .fabushiFont(.caption)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(theme.surfaceElevated, in: Capsule())
                .foregroundStyle(theme.textPrimary)
        }
        .fabushiMinimumTouchTarget()
    }

    // MARK: - Multi-Session PTY Tab

    private var ptyTerminalTab: some View {
        VStack(spacing: 0) {
            // Multi-session PTY tab strip
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(viewModel.ptyTabs) { tab in
                        let isSelected = tab.id == viewModel.activePtyTab?.id
                        HStack(spacing: 6) {
                            Button {
                                viewModel.selectedPtyTabId = tab.id
                            } label: {
                                Text(tab.title)
                                    .fabushiFont(.caption)
                                    .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textPrimary)
                            }
                            if viewModel.ptyTabs.count > 1 {
                                Button {
                                    viewModel.closePtySession(id: tab.id)
                                } label: {
                                    Image(systemName: "xmark")
                                        .fabushiFont(.caption2)
                                        .foregroundStyle(isSelected ? theme.accentForegroundColor : theme.textSecondary)
                                }
                            }
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(
                            isSelected ? theme.accentColor : theme.surfaceElevated,
                            in: Capsule()
                        )
                    }

                    Button {
                        viewModel.createPtySession(for: node)
                    } label: {
                        Label(
                            FabushiPlatformStrings.text(.remotePtyNewSession, locale: locale),
                            systemImage: "plus"
                        )
                        .fabushiFont(.caption)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(theme.surfaceSecondary, in: Capsule())
                    }
                    .fabushiMinimumTouchTarget()
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            }
            .background(theme.surfaceTopBar)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 6) {
                    let lines = viewModel.activePtyTab?.outputLines ?? []
                    if lines.isEmpty {
                        Text(FabushiPlatformStrings.text(.remotePtyOutputPlaceholder, locale: locale))
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textMuted)
                            .padding(.top, 8)
                    } else {
                        ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                            Text(line)
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textPrimary)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
                .padding(14)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(theme.surfaceSecondary)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.ptyTerminal)

            Divider()

            HStack(spacing: 10) {
                TextField(
                    FabushiPlatformStrings.text(.remotePtyInputPlaceholder, locale: locale),
                    text: $viewModel.ptyInputDraft
                )
                .fabushiFont(.monospacedCaption)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled(true)
                .focused($isPtyInputFocused)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Remote.ptyInputField)
                .onSubmit {
                    submitPtyInput()
                }

                Button {
                    submitPtyInput()
                } label: {
                    Text(FabushiPlatformStrings.text(.remotePtySendAction, locale: locale))
                        .fabushiFont(.subheadline)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 10)
                        .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                        .foregroundStyle(theme.accentForegroundColor)
                }
                .fabushiMinimumTouchTarget()
            }
            .padding(12)
            .background(theme.surfaceTopBar)
        }
    }

    private func submitPtyInput() {
        let cmd = viewModel.ptyInputDraft
        Task {
            await viewModel.sendPtyCommand(nodeId: node.nodeId, rawCommand: cmd)
        }
    }

    // MARK: - Remote Files Tab

    private var remoteFilesTab: some View {
        VStack(spacing: 10) {
            HStack(spacing: 8) {
                TextField(".", text: $remotePathInput)
                    .fabushiFont(.monospacedCaption)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))

                Button {
                    Task {
                        _ = await viewModel.refreshRemoteDirectory(nodeId: node.nodeId, path: remotePathInput)
                    }
                } label: {
                    Text(FabushiPlatformStrings.text(.refresh, locale: locale))
                        .fabushiFont(.subheadline)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 10)
                        .background(theme.accentColor, in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous))
                        .foregroundStyle(theme.accentForegroundColor)
                }
                .fabushiMinimumTouchTarget()
            }
            .padding([.horizontal, .top], 16)

            List {
                let entries = viewModel.remoteDirectoryEntriesByNodeId[node.nodeId] ?? []
                if entries.isEmpty {
                    Text(FabushiPlatformStrings.text(locale, en: "Tap Refresh to list files on this node.", zhHans: "点击刷新列出该远端设备的目录内容。"))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(entries, id: \.self) { entry in
                        Label(entry, systemImage: entry.hasSuffix("/") ? "folder.fill" : "doc.text")
                            .fabushiFont(.body)
                    }
                }
            }
            .listStyle(.insetGrouped)
        }
    }

    // MARK: - Per-Device Settings Tab

    private var deviceSettingsTab: some View {
        Form {
            Section(FabushiPlatformStrings.text(.remotePolicyTitle, locale: locale)) {
                Picker(
                    FabushiPlatformStrings.text(.remotePolicyTitle, locale: locale),
                    selection: $selectedPolicy
                ) {
                    ForEach(FabushiComputerRunPolicy.allCases) { policy in
                        Text(policy.displayTitle(locale: locale)).tag(policy)
                    }
                }

                Button {
                    Task {
                        await viewModel.updatePolicy(
                            nodeId: node.nodeId,
                            policy: FabushiUpdateComputerPolicyRequest(runPolicy: selectedPolicy),
                            locale: locale
                        )
                    }
                } label: {
                    Text(FabushiPlatformStrings.text(.save, locale: locale))
                        .fabushiFont(.headline)
                }
                .fabushiMinimumTouchTarget()
            }

            Section(FabushiPlatformStrings.text(.remoteWorkbenchTestConnection, locale: locale)) {
                Button {
                    Task {
                        _ = await viewModel.testConnection(nodeId: node.nodeId, locale: locale)
                    }
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.remoteWorkbenchTestConnection, locale: locale),
                        systemImage: "checkmark.shield"
                    )
                    .fabushiFont(.body)
                }
                .fabushiMinimumTouchTarget()
            }

            Section(FabushiPlatformStrings.text(.remoteWorkbenchWakeOnLan, locale: locale)) {
                TextField("AA:BB:CC:DD:EE:FF", text: $wolMacAddress)
                    .fabushiFont(.monospacedCaption)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                TextField("255.255.255.255", text: $wolBroadcastIP)
                    .fabushiFont(.monospacedCaption)
                    .keyboardType(.numbersAndPunctuation)
                Button {
                    Task {
                        await viewModel.sendWakeOnLan(
                            macAddress: wolMacAddress,
                            broadcastIP: wolBroadcastIP.isEmpty ? nil : wolBroadcastIP,
                            locale: locale
                        )
                    }
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.remoteWorkbenchWakeOnLan, locale: locale),
                        systemImage: "bolt.horizontal.circle"
                    )
                    .fabushiFont(.body)
                }
                .disabled(wolMacAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .fabushiMinimumTouchTarget()
            }

            Section {
                Button(role: .destructive) {
                    Task {
                        await viewModel.deleteComputer(nodeId: node.nodeId, locale: locale)
                    }
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.delete, locale: locale),
                        systemImage: "trash"
                    )
                    .fabushiFont(.body)
                }
                .fabushiMinimumTouchTarget()
            }
        }
    }
}
