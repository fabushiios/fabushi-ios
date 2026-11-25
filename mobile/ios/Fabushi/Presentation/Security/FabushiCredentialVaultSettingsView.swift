import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Zero-Knowledge Credential Vault & RFC 6238 TOTP Settings View (F23 UI)

@MainActor
public struct FabushiCredentialVaultSettingsView: View {
    public let coordinator: FabushiCoordinator

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(ShellNavigationRouter.self) private var injectedRouter: ShellNavigationRouter?
    @State private var viewModel: FabushiCredentialVaultViewModel

    // Reveal & Modal States
    @State private var revealedItemIds: Set<String> = []
    @State private var showingAddItemSheet = false
    @State private var newItemKey = ""
    @State private var newItemPlaintext = ""
    @State private var showingAddTotpSheet = false
    @State private var newTotpLabel = ""
    @State private var newTotpIssuer = ""
    @State private var newTotpSecret = ""
    @State private var showingTotpQrScanner = false
    @State private var showingUnlockOptionsSheet = false
    @State private var unlockPassphraseInput = ""
    @State private var unlockEmergencyKitInput = ""
    @State private var showingDeviceSyncSheet = false
    @State private var hasAcknowledgedEmergencyKit = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        securityKeychain: FabushiSecurityKeychainStore = .shared
    ) {
        self.coordinator = coordinator
        self._viewModel = State(
            initialValue: FabushiCredentialVaultViewModel(
                coordinator: coordinator,
                securityKeychain: securityKeychain
            )
        )
    }

    public var body: some View {
        List {
            vaultLockControlSection
            if viewModel.isUnlocked {
                totpAuthenticatorSection
                encryptedItemsSection
            }
            deviceSyncSection
            obscuraSandboxNoticeSection
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.vault"))
        .task {
            await viewModel.refreshVaultStatus()
        }
        .onDisappear {
            revealedItemIds.removeAll()
            viewModel.stopTotpTicker()
        }
        .sheet(isPresented: $showingAddItemSheet) {
            addEncryptedItemSheet
        }
        .sheet(isPresented: $showingAddTotpSheet) {
            addTotpSheet
        }
        .sheet(isPresented: $showingTotpQrScanner) {
            FabushiQRScannerSheet(
                titleKey: "hig.vault.scanTotpQr",
                manualPlaceholderKey: "hig.vault.totpSecretPlaceholder",
                onScannedPayload: { payload in
                    showingTotpQrScanner = false
                    newTotpSecret = payload
                    showingAddTotpSheet = true
                },
                onCancel: {
                    showingTotpQrScanner = false
                }
            )
        }
        .sheet(isPresented: $showingUnlockOptionsSheet) {
            advancedUnlockSheet
        }
        .sheet(isPresented: $showingDeviceSyncSheet) {
            deviceSyncModalSheet
        }
        .sheet(
            isPresented: Binding(
                get: { viewModel.pendingEmergencyKit != nil },
                set: { _ in }
            )
        ) {
            emergencyKitBackupSheet
        }
    }

    // MARK: - Vault Status & Lock/Unlock CTA

    private var vaultLockControlSection: some View {
        Section(header: Text(theme.localized("hig.vault.statusHeader"))) {
            HStack(spacing: 12) {
                Image(systemName: viewModel.isUnlocked ? "lock.open.fill" : "lock.fill")
                    .fabushiFont(.title3)
                    .foregroundStyle(viewModel.isUnlocked ? theme.successColor : theme.accentColor)
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 2) {
                    Text(
                        viewModel.isUnlocked
                            ? theme.localized("hig.vault.unlockedStatus")
                            : theme.localized("hig.vault.lockedStatus")
                    )
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)

                    Text(theme.localized("hig.vault.zeroKnowledgeSubtitle"))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                }
            }
            .padding(.vertical, 4)

            if viewModel.isUnlocked {
                FabushiGlassPillButton(
                    title: theme.localized("hig.vault.syncNowButton"),
                    systemImage: "arrow.triangle.2.circlepath",
                    accessibilityID: FabushiAccessibilityIDs.Settings.vaultUnlockButton,
                    isProminent: false
                ) {
                    Task {
                        await viewModel.loadUnlockedVaultContents()
                    }
                }
                .frame(maxWidth: .infinity)
                .disabled(viewModel.isBusy)

                Button(role: .destructive) {
                    revealedItemIds.removeAll()
                    Task {
                        await viewModel.lockVault()
                    }
                } label: {
                    Label(theme.localized("hig.vault.lockButton"), systemImage: "lock.fill")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.destructiveColor)
                        .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(viewModel.isBusy)
                .accessibilityIdentifier("fabushi.internal.vault.lockButton")
            } else {
                FabushiGlassPillButton(
                    title: theme.localized("hig.vault.unlockButton"),
                    systemImage: "lock.open.fill",
                    accessibilityID: FabushiAccessibilityIDs.Settings.vaultUnlockButton,
                    isProminent: true
                ) {
                    Task {
                        await viewModel.unlockVault()
                        if !viewModel.isUnlocked, viewModel.unlockPromptReason != nil {
                            unlockPassphraseInput = ""
                            unlockEmergencyKitInput = ""
                            showingUnlockOptionsSheet = true
                        }
                    }
                }
                .frame(maxWidth: .infinity)
                .disabled(viewModel.isBusy)

                Button {
                    unlockPassphraseInput = ""
                    unlockEmergencyKitInput = ""
                    showingUnlockOptionsSheet = true
                } label: {
                    Label(theme.localized("hig.vault.unlockWithKeyOrKit"), systemImage: "key.viewfinder")
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.accentColor)
                        .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }

            if let errorMessage = viewModel.errorMessage {
                Text(errorMessage)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.destructiveColor)
            }
        }
    }

    // MARK: - RFC 6238 TOTP Authenticator Section (`fabushi.settings.vaultTotpCodeLabel`)

    private var totpAuthenticatorSection: some View {
        Section(header: Text(theme.localized("hig.vault.totpHeader"))) {
            if let primaryTotp = viewModel.totpAccounts.first {
                let liveCode = viewModel.liveTotpCodes[primaryTotp.id] ?? "------"
                totpEntryRow(
                    entry: primaryTotp,
                    code: liveCode,
                    isPrimaryForAccessibility: true
                )

                ForEach(Array(viewModel.totpAccounts.dropFirst())) { entry in
                    totpEntryRow(
                        entry: entry,
                        code: viewModel.liveTotpCodes[entry.id] ?? "------",
                        isPrimaryForAccessibility: false
                    )
                }
            } else {
                // Honest unlocked TOTP status surface (always renders `vaultTotpCodeLabel` when unlocked)
                let summaryText = viewModel.remoteEncryptedTotpCount > 0
                    ? "\(theme.localized("hig.vault.totpSyncedRemoteReady")) (\(viewModel.remoteEncryptedTotpCount))"
                    : theme.localized("hig.vault.totpReadyEmpty")
                HStack(spacing: 12) {
                    totpCountdownRing
                    VStack(alignment: .leading, spacing: 2) {
                        Text(summaryText)
                            .fabushiFont(.callout)
                            .foregroundStyle(theme.textPrimary)
                            .accessibilityLabel(summaryText)
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.vaultTotpCodeLabel)
                        Text(theme.localized("hig.vault.totpAddHint"))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.textSecondary)
                    }
                }
                .padding(.vertical, 4)
            }

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    addTotpManualButton
                    scanTotpQrButton
                }
                VStack(spacing: 8) {
                    addTotpManualButton
                    scanTotpQrButton
                }
            }
        }
    }

    private func totpEntryRow(
        entry: FabushiStoredTotpAccount,
        code: String,
        isPrimaryForAccessibility: Bool
    ) -> some View {
        HStack(spacing: 12) {
            totpCountdownRing

            VStack(alignment: .leading, spacing: 3) {
                Text(entry.issuer.isEmpty ? entry.label : "\(entry.issuer) · \(entry.label)")
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)

                let formattedCode = formatTotpDigits(code)
                let labelView = Text(formattedCode)
                    .fabushiFont(.title2)
                    .fontWeight(.semibold)
                    .foregroundStyle(theme.textPrimary)
                    .accessibilityLabel("\(entry.label), \(code), \(viewModel.totpRemainingSeconds)s")

                if isPrimaryForAccessibility {
                    labelView
                        .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.vaultTotpCodeLabel)
                } else {
                    labelView
                }
            }

            Spacer(minLength: 8)

            Button {
                copyToClipboard(code, toastKey: "hig.vault.totpCopiedToast")
            } label: {
                Image(systemName: "doc.on.doc")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
            }
            .buttonStyle(.plain)
            .accessibilityLabel(theme.localized("hig.vault.copyCode"))
        }
        .padding(.vertical, 4)
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            Button(role: .destructive) {
                try? viewModel.deleteTotpAccount(id: entry.id)
            } label: {
                Label(theme.localized("hig.common.close"), systemImage: "trash")
            }
        }
    }

    private var totpCountdownRing: some View {
        let progress = Double(viewModel.totpRemainingSeconds) / 30.0
        return ZStack {
            Circle()
                .stroke(theme.surfaceInput, lineWidth: 3)
            Circle()
                .trim(from: 0, to: min(max(progress, 0), 1))
                .stroke(
                    viewModel.totpRemainingSeconds <= 5 ? theme.warningColor : theme.accentColor,
                    style: StrokeStyle(lineWidth: 3, lineCap: .round)
                )
                .rotationEffect(.degrees(-90))
            Text("\(viewModel.totpRemainingSeconds)")
                .fabushiFont(.caption2)
                .foregroundStyle(theme.textSecondary)
        }
        .frame(width: 34, height: 34)
        .accessibilityHidden(true)
    }

    private var addTotpManualButton: some View {
        Button {
            newTotpLabel = ""
            newTotpIssuer = ""
            newTotpSecret = ""
            showingAddTotpSheet = true
        } label: {
            Label(theme.localized("hig.vault.addTotp"), systemImage: "plus.circle.fill")
                .fabushiFont(.callout)
                .foregroundStyle(theme.accentColor)
                .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var scanTotpQrButton: some View {
        Button {
            showingTotpQrScanner = true
        } label: {
            Label(theme.localized("hig.vault.scanTotpQr"), systemImage: "qrcode.viewfinder")
                .fabushiFont(.callout)
                .foregroundStyle(theme.accentColor)
                .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    // MARK: - Encrypted Categories & Items CRUD (`password`, `browser_cookie`, `ai_oauth`)

    private var encryptedItemsSection: some View {
        Section(header: Text(theme.localized("hig.vault.itemsHeader"))) {
            Picker(
                theme.localized("hig.vault.categoryPickerLabel"),
                selection: $viewModel.selectedCategory
            ) {
                ForEach(FabushiVaultCategory.allCases) { category in
                    Text(theme.localized(category.localizationKey)).tag(category)
                }
            }
            .pickerStyle(.segmented)

            if viewModel.filteredItems.isEmpty {
                Text(theme.localized("hig.vault.noItemsInCategory"))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
            } else {
                ForEach(viewModel.filteredItems) { item in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text(item.itemKey)
                                .fabushiFont(.headline)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            Text("r\(item.revision)")
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textMuted)
                        }

                        if let plaintext = item.plaintext {
                            let isRevealed = revealedItemIds.contains(item.id)
                            Text(isRevealed ? plaintext : "••••••••••••")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(isRevealed ? nil : 1)

                            HStack(spacing: 12) {
                                Button {
                                    if isRevealed {
                                        revealedItemIds.remove(item.id)
                                    } else {
                                        revealedItemIds.insert(item.id)
                                    }
                                } label: {
                                    Text(
                                        isRevealed
                                            ? theme.localized("hig.vault.hideSecret")
                                            : theme.localized("hig.vault.revealSecret")
                                    )
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.accentColor)
                                    .fabushiMinimumTouchTarget()
                                }
                                .buttonStyle(.plain)

                                Button {
                                    copyToClipboard(plaintext, toastKey: "hig.vault.secretCopiedToast")
                                } label: {
                                    Text(theme.localized("hig.vault.copySecret"))
                                        .fabushiFont(.caption)
                                        .foregroundStyle(theme.accentColor)
                                        .fabushiMinimumTouchTarget()
                                }
                                .buttonStyle(.plain)
                            }
                        } else {
                            Text(theme.localized("hig.vault.encryptedRemoteItem"))
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textMuted)
                        }
                    }
                    .padding(.vertical, 4)
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button(role: .destructive) {
                            Task {
                                do {
                                    try await viewModel.deleteEncryptedItem(item)
                                } catch {
                                    viewModel.errorMessage = error.localizedDescription
                                }
                            }
                        } label: {
                            Label(theme.localized("hig.common.close"), systemImage: "trash")
                        }
                    }
                }
            }

            Button {
                newItemKey = ""
                newItemPlaintext = ""
                showingAddItemSheet = true
            } label: {
                Label(theme.localized("hig.vault.addItem"), systemImage: "plus.circle.fill")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: - X25519 Cross-Device Sync Approval

    private var deviceSyncSection: some View {
        Section(header: Text(theme.localized("hig.vault.deviceSyncHeader"))) {
            Button {
                showingDeviceSyncSheet = true
                if viewModel.isUnlocked {
                    Task { try? await viewModel.loadPendingDeviceSyncRequests() }
                }
            } label: {
                Label(theme.localized("hig.vault.openDeviceSync"), systemImage: "arrow.triangle.2.circlepath.circle.fill")
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: - Honest iOS Sandbox Capability Notice for Remote Obscura

    private var obscuraSandboxNoticeSection: some View {
        Section(header: Text(theme.localized("hig.vault.obscuraHeader"))) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "info.circle.fill")
                    .fabushiFont(.title3)
                    .foregroundStyle(theme.warningColor)
                    .accessibilityHidden(true)

                Text(theme.localized("hig.vault.obscuraIosSandboxNotice"))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(.vertical, 4)
        }
    }

    // MARK: - Modal Sheets

    private var addEncryptedItemSheet: some View {
        NavigationStack {
            Form {
                Section(header: Text(theme.localized(viewModel.selectedCategory.localizationKey))) {
                    TextField(theme.localized("hig.vault.itemKeyPlaceholder"), text: $newItemKey)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField(theme.localized("hig.vault.itemPlaintextPlaceholder"), text: $newItemPlaintext, axis: .vertical)
                        .fabushiFont(.body)
                        .lineLimit(3...8)
                }
                Section {
                    Text(theme.localized("hig.vault.payloadLimitNote"))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
            }
            .navigationTitle(theme.localized("hig.vault.addItem"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        showingAddItemSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done")) {
                        Task {
                            do {
                                _ = try await viewModel.saveEncryptedItem(
                                    category: viewModel.selectedCategory,
                                    itemKey: newItemKey,
                                    plaintext: newItemPlaintext
                                )
                                showingAddItemSheet = false
                            } catch {
                                viewModel.errorMessage = error.localizedDescription
                                showingAddItemSheet = false
                            }
                        }
                    }
                    .disabled(newItemKey.isEmpty || newItemPlaintext.isEmpty)
                }
            }
        }
    }

    private var addTotpSheet: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(theme.localized("hig.vault.totpLabelPlaceholder"), text: $newTotpLabel)
                        .fabushiFont(.body)
                    TextField(theme.localized("hig.vault.totpIssuerPlaceholder"), text: $newTotpIssuer)
                        .fabushiFont(.body)
                    TextField(theme.localized("hig.vault.totpSecretPlaceholder"), text: $newTotpSecret)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
            }
            .navigationTitle(theme.localized("hig.vault.addTotp"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        showingAddTotpSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done")) {
                        Task {
                            do {
                                try await viewModel.addTotpAccount(
                                    label: newTotpLabel,
                                    issuer: newTotpIssuer,
                                    secretOrURI: newTotpSecret
                                )
                                showingAddTotpSheet = false
                            } catch {
                                viewModel.errorMessage = error.localizedDescription
                                showingAddTotpSheet = false
                            }
                        }
                    }
                    .disabled(newTotpSecret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }

    private var advancedUnlockSheet: some View {
        let trimmedKit = unlockEmergencyKitInput.trimmingCharacters(in: .whitespacesAndNewlines)
        let canSubmit = !unlockPassphraseInput.isEmpty || !trimmedKit.isEmpty
        return NavigationStack {
            Form {
                if let promptReason = viewModel.unlockPromptReason {
                    Section {
                        Text(
                            promptReason == .noRemoteEnvelopeRequiresInitialization
                                ? theme.localized("hig.vault.initPassphrasePrompt")
                                : theme.localized("hig.vault.existingEnvelopePrompt")
                        )
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)

                        if promptReason == .existingRemoteEnvelopeRequiresCredentials {
                            Button {
                                showingUnlockOptionsSheet = false
                                showingDeviceSyncSheet = true
                            } label: {
                                Label(
                                    theme.localized("hig.vault.openDeviceSync"),
                                    systemImage: "arrow.triangle.2.circlepath.circle.fill"
                                )
                                .fabushiFont(.body)
                            }
                        }
                    }
                }

                Section(header: Text(theme.localized("hig.vault.unlockPassphraseHeader"))) {
                    SecureField(theme.localized("hig.auth.passwordPlaceholder"), text: $unlockPassphraseInput)
                        .fabushiFont(.body)
                }
                Section(header: Text(theme.localized("hig.vault.unlockEmergencyKitHeader"))) {
                    TextField(theme.localized("hig.vault.emergencyKitPlaceholder"), text: $unlockEmergencyKitInput)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                }
            }
            .navigationTitle(theme.localized("hig.vault.unlockButton"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        unlockPassphraseInput = ""
                        unlockEmergencyKitInput = ""
                        showingUnlockOptionsSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.vault.unlockButton")) {
                        let pass = unlockPassphraseInput.isEmpty ? nil : unlockPassphraseInput
                        let kit = trimmedKit.isEmpty ? nil : trimmedKit
                        unlockPassphraseInput = ""
                        unlockEmergencyKitInput = ""
                        showingUnlockOptionsSheet = false
                        Task {
                            await viewModel.unlockVault(
                                passphraseOrPassword: pass,
                                emergencyKit: kit
                            )
                        }
                    }
                    .disabled(!canSubmit || viewModel.isBusy)
                }
            }
        }
    }

    private var emergencyKitBackupSheet: some View {
        NavigationStack {
            Form {
                if let kit = viewModel.pendingEmergencyKit {
                    Section {
                        Text(theme.localized("hig.vault.emergencyKitWarning"))
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textSecondary)

                        Text(kit)
                            .fabushiFont(.monospacedBody)
                            .foregroundStyle(theme.textPrimary)
                            .textSelection(.enabled)
                            .padding(.vertical, 6)
                            .accessibilityLabel(kit)
                            .accessibilityIdentifier("fabushi.internal.vault.emergencyKitValue")

                        Button {
                            copyToClipboard(kit, toastKey: "hig.vault.emergencyKitCopiedToast")
                        } label: {
                            Label(theme.localized("hig.vault.emergencyKitCopyButton"), systemImage: "doc.on.doc")
                                .fabushiFont(.body)
                        }
                        .accessibilityIdentifier("fabushi.internal.vault.emergencyKitCopyButton")
                    }

                    Section {
                        Toggle(
                            theme.localized("hig.vault.emergencyKitSavedAck"),
                            isOn: $hasAcknowledgedEmergencyKit
                        )
                        .accessibilityIdentifier("fabushi.internal.vault.emergencyKitAckToggle")
                    }
                }
            }
            .navigationTitle(theme.localized("hig.vault.emergencyKitTitle"))
            .navigationBarTitleDisplayMode(.inline)
            .interactiveDismissDisabled(true)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done")) {
                        hasAcknowledgedEmergencyKit = false
                        viewModel.acknowledgeEmergencyKit()
                    }
                    .disabled(!hasAcknowledgedEmergencyKit)
                    .accessibilityIdentifier("fabushi.internal.vault.emergencyKitDoneButton")
                }
            }
        }
    }

    private var deviceSyncModalSheet: some View {
        NavigationStack {
            List {
                Section(header: Text(theme.localized("hig.vault.deviceSyncRequestHeader"))) {
                    if let code = viewModel.activeSyncVerificationCode {
                        HStack(spacing: 12) {
                            VStack(alignment: .leading, spacing: 6) {
                                Text(theme.localized("hig.vault.safetyFingerprintLabel"))
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.textSecondary)
                                Text(code)
                                    .fabushiFont(.title2)
                                    .fontWeight(.bold)
                                    .foregroundStyle(theme.accentColor)
                            }
                            Spacer(minLength: 8)
                            Button {
                                copyToClipboard(code, toastKey: "hig.vault.safetyCodeCopiedToast")
                            } label: {
                                Image(systemName: "doc.on.doc")
                                    .fabushiFont(.body)
                                    .foregroundStyle(theme.accentColor)
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(theme.localized("hig.vault.copySecret"))
                        }
                        .padding(.vertical, 4)

                        Button(theme.localized("hig.vault.deviceSyncCheckStatus")) {
                            Task { _ = try? await viewModel.checkDeviceSyncStatusAndOpenIfApproved() }
                        }
                    }

                    Button(theme.localized("hig.vault.deviceSyncStartRequest")) {
                        Task { _ = try? await viewModel.startDeviceSyncRequest() }
                    }
                }

                if viewModel.isUnlocked {
                    Section(header: Text(theme.localized("hig.vault.deviceSyncPendingHeader"))) {
                        if viewModel.pendingDeviceSyncRequests.isEmpty {
                            Text(theme.localized("hig.vault.deviceSyncNoPending"))
                                .fabushiFont(.footnote)
                                .foregroundStyle(theme.textSecondary)
                        } else {
                            ForEach(viewModel.pendingDeviceSyncRequests) { pending in
                                VStack(alignment: .leading, spacing: 8) {
                                    Text(pending.deviceName)
                                        .fabushiFont(.headline)
                                        .foregroundStyle(theme.textPrimary)
                                    Text("\(theme.localized("hig.vault.safetyFingerprintLabel")): \(pending.verificationCode)")
                                        .fabushiFont(.monospacedCaption)
                                        .foregroundStyle(theme.accentColor)

                                    HStack(spacing: 12) {
                                        Button(theme.localized("hig.vault.deviceSyncApprove")) {
                                            Task { _ = try? await viewModel.approvePendingDeviceSync(pending) }
                                        }
                                        .buttonStyle(.borderedProminent)

                                        Button(theme.localized("hig.vault.deviceSyncDeny"), role: .destructive) {
                                            Task { _ = try? await viewModel.denyPendingDeviceSync(pending) }
                                        }
                                        .buttonStyle(.bordered)
                                    }
                                }
                                .padding(.vertical, 4)
                            }
                        }
                    }
                }
            }
            .navigationTitle(theme.localized("hig.vault.deviceSyncHeader"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done")) {
                        showingDeviceSyncSheet = false
                    }
                }
            }
        }
    }

    private func formatTotpDigits(_ raw: String) -> String {
        guard raw.count == 6 else { return raw }
        let mid = raw.index(raw.startIndex, offsetBy: 3)
        return "\(raw[..<mid]) \(raw[mid...])"
    }

    private func copyToClipboard(_ value: String, toastKey: String) {
        #if canImport(UIKit)
        FabushiSecureClipboard.copySensitive(value)
        #endif
        injectedRouter?.toastController.showStagePill(theme.localized(toastKey))
    }
}
