import SwiftUI

// MARK: - Privacy & Security Hub View (F04 Passkeys/QR Pair, F05 Cloud 2FA, F06 Passcode/Biometrics)

@MainActor
public struct FabushiPrivacySecuritySettingsView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let passcodeController: FabushiPasscodeLockController

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(FabushiFeatureModuleRegistry.self) private var injectedRegistry: FabushiFeatureModuleRegistry?

    // Passcode setup & disable confirmation sheets (D5)
    @State private var showingPasscodeSheet = false
    @State private var currentPasscodeInput = ""
    @State private var newPasscodeInput = ""
    @State private var confirmPasscodeInput = ""
    @State private var passcodeStatusError: String?
    @State private var showingPasscodeDisableSheet = false
    @State private var disablePasscodeInput = ""
    @State private var disablePasscodeError: String?

    // Cloud 2FA (F05)
    @State private var twoFactorStatus: FabushiTwoFactorStatusResponse?
    @State private var showingTwoFactorSetSheet = false
    @State private var showingTwoFactorDisableSheet = false
    @State private var twoFactorCurrentPassword = ""
    @State private var twoFactorNewPassword = ""
    @State private var twoFactorHint = ""
    @State private var twoFactorRecoveryEmail = ""
    @State private var twoFactorError: String?

    // Passkeys & QR Pair Accept (F04)
    @State private var passkeys: [FabushiPasskeyRecordDTO] = []
    @State private var passkeyStatusMessage: String?
    @State private var showingQrPairScanner = false
    @State private var qrPairStatusMessage: String?
    @State private var isWorking = false

    @State private var passkeyController = FabushiWebAuthnPasskeyController()

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var registry: FabushiFeatureModuleRegistry { injectedRegistry ?? .shared }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        passcodeController: FabushiPasscodeLockController = .shared
    ) {
        self.coordinator = coordinator
        self.router = router
        self.passcodeController = passcodeController
    }

    public var body: some View {
        let biometry = passcodeController.canEvaluateBiometrics()

        List {
            // Section 1: Local Passcode Lock, Biometrics & Auto-Lock (F06)
            Section(header: Text(theme.localized("hig.passcode.sectionHeader"))) {
                HStack(spacing: 12) {
                    Label(theme.localized("hig.auth.passcodeTitle"), systemImage: "lock.fill")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    Text(
                        passcodeController.isPasscodeEnabled
                            ? theme.localized("hig.security.enabled")
                            : theme.localized("hig.security.disabled")
                    )
                    .fabushiFont(.footnote)
                    .foregroundStyle(passcodeController.isPasscodeEnabled ? theme.successColor : theme.textSecondary)
                }
                .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)

                Button {
                    currentPasscodeInput = ""
                    newPasscodeInput = ""
                    confirmPasscodeInput = ""
                    passcodeStatusError = nil
                    showingPasscodeSheet = true
                } label: {
                    Label(
                        passcodeController.isPasscodeEnabled
                            ? theme.localized("hig.passcode.changeButton")
                            : theme.localized("hig.passcode.enableButton"),
                        systemImage: "key.fill"
                    )
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if passcodeController.isPasscodeEnabled {
                    Toggle(
                        isOn: Binding(
                            get: { passcodeController.biometricsEnabled },
                            set: { newValue in
                                try? passcodeController.setBiometricsEnabled(newValue)
                            }
                        )
                    ) {
                        Label(
                            theme.localized(biometry.kind.localizationKey),
                            systemImage: biometry.kind.systemImage
                        )
                        .fabushiFont(.body)
                    }
                    .disabled(!biometry.available)

                    Picker(
                        theme.localized("hig.passcode.autoLockTitle"),
                        selection: Binding(
                            get: {
                                FabushiAutoLockPreset(rawValue: passcodeController.autoLockTimeoutSeconds)
                                    ?? .fiveMinutes
                            },
                            set: { preset in
                                try? passcodeController.setAutoLockTimeoutSeconds(preset.rawValue)
                            }
                        )
                    ) {
                        ForEach(FabushiAutoLockPreset.allCases) { preset in
                            Text(theme.localized(preset.localizationKey)).tag(preset)
                        }
                    }
                    .fabushiFont(.body)

                    Button {
                        router.isPasscodeLockEnabled = true
                        router.engagePasscodeLockIfEnabled()
                    } label: {
                        Label(theme.localized("hig.passcode.lockNow"), systemImage: "lock.rotation")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)

                    Button(role: .destructive) {
                        disablePasscodeInput = ""
                        disablePasscodeError = nil
                        showingPasscodeDisableSheet = true
                    } label: {
                        Label(theme.localized("hig.passcode.disableButton"), systemImage: "lock.slash")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.destructiveColor)
                            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }

                if let passcodeStatusError {
                    Text(passcodeStatusError)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.destructiveColor)
                }
            }

            // Section 2: Cloud 2FA Password (F05)
            Section(header: Text(theme.localized("hig.security.twoFactorHeader"))) {
                HStack(spacing: 12) {
                    Label(theme.localized("hig.auth.twoFactorTitle"), systemImage: "key.viewfinder")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Spacer()
                    Text(
                        twoFactorStatus?.hasPassword == true
                            ? theme.localized("hig.security.enabled")
                            : theme.localized("hig.security.disabled")
                    )
                    .fabushiFont(.footnote)
                    .foregroundStyle(twoFactorStatus?.hasPassword == true ? theme.successColor : theme.textSecondary)
                }
                .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)

                if let hint = twoFactorStatus?.hint, !hint.isEmpty {
                    Text("\(theme.localized("hig.auth.twoFactorHintPrefix")) \(hint)")
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                }

                Button {
                    twoFactorCurrentPassword = ""
                    twoFactorNewPassword = ""
                    twoFactorHint = twoFactorStatus?.hint ?? ""
                    twoFactorRecoveryEmail = ""
                    twoFactorError = nil
                    showingTwoFactorSetSheet = true
                } label: {
                    Label(
                        twoFactorStatus?.hasPassword == true
                            ? theme.localized("hig.security.changeTwoFactor")
                            : theme.localized("hig.security.enableTwoFactor"),
                        systemImage: "shield.lefthalf.filled"
                    )
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if twoFactorStatus?.hasPassword == true {
                    Button(role: .destructive) {
                        twoFactorCurrentPassword = ""
                        twoFactorError = nil
                        showingTwoFactorDisableSheet = true
                    } label: {
                        Label(theme.localized("hig.security.disableTwoFactor"), systemImage: "shield.slash")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.destructiveColor)
                            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }

                if let twoFactorError {
                    Text(twoFactorError)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.destructiveColor)
                }
            }

            // Section 3: Passkeys & Cross-Device QR Pairing (F04)
            Section(header: Text(theme.localized("hig.security.passkeysAndPairingHeader"))) {
                ForEach(passkeys) { item in
                    HStack(spacing: 10) {
                        Image(systemName: "person.badge.key.fill")
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.deviceName)
                                .fabushiFont(.body)
                                .foregroundStyle(theme.textPrimary)
                            Text(item.credentialId)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                                .lineLimit(1)
                        }
                        Spacer()
                        Button(role: .destructive) {
                            Task { await deletePasskey(id: item.id) }
                        } label: {
                            Image(systemName: "trash")
                                .foregroundStyle(theme.destructiveColor)
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(theme.localized("hig.security.deletePasskey"))
                    }
                }

                Button {
                    Task { await registerNewPasskey() }
                } label: {
                    Label(theme.localized("hig.security.addPasskey"), systemImage: "plus.circle.fill")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(isWorking)

                if let passkeyStatusMessage {
                    Text(passkeyStatusMessage)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                }

                Button {
                    qrPairStatusMessage = nil
                    showingQrPairScanner = true
                } label: {
                    Label(theme.localized("hig.security.scanQrToPair"), systemImage: "qrcode.viewfinder")
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if let qrPairStatusMessage {
                    Text(qrPairStatusMessage)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                }
            }

            // Section 4: Credential Vault & Active Devices Links
            Section(header: Text(theme.localized("hig.security.storageAndSessionsHeader"))) {
                NavigationLink {
                    registry.security.makeVaultSettingsView(coordinator: coordinator)
                } label: {
                    Label(theme.localized("hig.settings.vault"), systemImage: "key.fill")
                        .fabushiFont(.body)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                }

                NavigationLink {
                    registry.security.makeDevicesSettingsView(coordinator: coordinator)
                } label: {
                    Label(theme.localized("hig.settings.devices"), systemImage: "laptopcomputer.and.iphone")
                        .fabushiFont(.body)
                        .frame(minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.privacySecurity"))
        .task {
            await loadSecurityStatus()
        }
        .sheet(isPresented: $showingPasscodeSheet) {
            passcodeConfigSheet
        }
        .sheet(isPresented: $showingPasscodeDisableSheet) {
            passcodeDisableSheet
        }
        .sheet(isPresented: $showingTwoFactorSetSheet) {
            twoFactorSetSheet
        }
        .sheet(isPresented: $showingTwoFactorDisableSheet) {
            twoFactorDisableSheet
        }
        .sheet(isPresented: $showingQrPairScanner) {
            FabushiQRScannerSheet(
                titleKey: "hig.security.scanQrToPair",
                manualPlaceholderKey: "hig.security.qrPairManualPlaceholder",
                onScannedPayload: { payload in
                    showingQrPairScanner = false
                    Task { await acceptQrPairing(payload: payload) }
                },
                onCancel: {
                    showingQrPairScanner = false
                }
            )
        }
    }

    // MARK: - Sub-Sheets

    private var passcodeConfigSheet: some View {
        let biometry = passcodeController.canEvaluateBiometrics()
        return NavigationStack {
            Form {
                if passcodeController.isPasscodeEnabled {
                    Section(header: Text(theme.localized("hig.passcode.verifyCurrentHeader"))) {
                        SecureField(theme.localized("hig.passcode.currentPlaceholder"), text: $currentPasscodeInput)
                            .fabushiFont(.body)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()

                        if passcodeController.biometricsEnabled && biometry.available {
                            Button {
                                Task { await saveLocalPasscode(useBiometrics: true) }
                            } label: {
                                Label(
                                    theme.localized("hig.passcode.verifyWithBiometrics"),
                                    systemImage: biometry.kind.systemImage
                                )
                                .fabushiFont(.body)
                            }
                            .disabled(isWorking)
                        }
                    }
                }

                Section(header: Text(theme.localized("hig.passcode.setupHeader"))) {
                    SecureField(theme.localized("hig.passcode.newPlaceholder"), text: $newPasscodeInput)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField(theme.localized("hig.passcode.confirmPlaceholder"), text: $confirmPasscodeInput)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.done)
                        .onSubmit {
                            if !isWorking {
                                Task { await saveLocalPasscode(useBiometrics: false) }
                            }
                        }
                }

                if let passcodeStatusError {
                    Section {
                        Text(passcodeStatusError)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
            }
            .navigationTitle(theme.localized("hig.auth.passcodeTitle"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        currentPasscodeInput = ""
                        newPasscodeInput = ""
                        confirmPasscodeInput = ""
                        showingPasscodeSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done")) {
                        Task { await saveLocalPasscode(useBiometrics: false) }
                    }
                    .disabled(isWorking)
                }
            }
            .task {
                _ = await passcodeController.syncPasscodeStatus(coordinator: coordinator)
            }
        }
    }

    private var passcodeDisableSheet: some View {
        let biometry = passcodeController.canEvaluateBiometrics()
        return NavigationStack {
            Form {
                Section(header: Text(theme.localized("hig.passcode.disableConfirmHeader"))) {
                    SecureField(theme.localized("hig.passcode.currentPlaceholder"), text: $disablePasscodeInput)
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.done)
                        .onSubmit {
                            if !isWorking && !disablePasscodeInput.isEmpty {
                                Task { await removeLocalPasscode(useBiometrics: false) }
                            }
                        }

                    if passcodeController.biometricsEnabled && biometry.available {
                        Button(role: .destructive) {
                            Task { await removeLocalPasscode(useBiometrics: true) }
                        } label: {
                            Label(
                                theme.localized("hig.passcode.verifyWithBiometrics"),
                                systemImage: biometry.kind.systemImage
                            )
                            .fabushiFont(.body)
                        }
                        .disabled(isWorking)
                    }
                }

                if let disablePasscodeError {
                    Section {
                        Text(disablePasscodeError)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
            }
            .navigationTitle(theme.localized("hig.passcode.disableButton"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        disablePasscodeInput = ""
                        disablePasscodeError = nil
                        showingPasscodeDisableSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done"), role: .destructive) {
                        Task { await removeLocalPasscode(useBiometrics: false) }
                    }
                    .disabled(isWorking || disablePasscodeInput.isEmpty)
                }
            }
        }
    }

    private var twoFactorSetSheet: some View {
        NavigationStack {
            Form {
                Section {
                    if twoFactorStatus?.hasPassword == true {
                        SecureField(
                            theme.localized("hig.security.currentTwoFactorPlaceholder"),
                            text: $twoFactorCurrentPassword
                        )
                        .fabushiFont(.body)
                    }
                    SecureField(
                        theme.localized("hig.security.newTwoFactorPlaceholder"),
                        text: $twoFactorNewPassword
                    )
                    .fabushiFont(.body)
                    TextField(
                        theme.localized("hig.security.twoFactorHintPlaceholder"),
                        text: $twoFactorHint
                    )
                    .fabushiFont(.body)
                    TextField(
                        theme.localized("hig.security.twoFactorRecoveryEmailPlaceholder"),
                        text: $twoFactorRecoveryEmail
                    )
                    .fabushiFont(.body)
                    .keyboardType(.emailAddress)
                    .textInputAutocapitalization(.never)
                }

                if let twoFactorError {
                    Section {
                        Text(twoFactorError)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
            }
            .navigationTitle(theme.localized("hig.auth.twoFactorTitle"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        showingTwoFactorSetSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done")) {
                        Task { await saveTwoFactorPassword() }
                    }
                    .disabled(isWorking || twoFactorNewPassword.isEmpty)
                }
            }
        }
    }

    private var twoFactorDisableSheet: some View {
        NavigationStack {
            Form {
                Section(header: Text(theme.localized("hig.security.disableTwoFactorPrompt"))) {
                    SecureField(
                        theme.localized("hig.security.currentTwoFactorPlaceholder"),
                        text: $twoFactorCurrentPassword
                    )
                    .fabushiFont(.body)
                }

                if let twoFactorError {
                    Section {
                        Text(twoFactorError)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                    }
                }
            }
            .navigationTitle(theme.localized("hig.security.disableTwoFactor"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        showingTwoFactorDisableSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(theme.localized("hig.common.done"), role: .destructive) {
                        Task { await disableTwoFactorPassword() }
                    }
                    .disabled(isWorking || twoFactorCurrentPassword.isEmpty)
                }
            }
        }
    }

    // MARK: - Actions

    private func loadSecurityStatus() async {
        guard coordinator.isStarted else { return }
        _ = await passcodeController.syncPasscodeStatus(coordinator: coordinator)
        if passcodeController.isPasscodeEnabled {
            router.isPasscodeLockEnabled = true
        }
        if let status: FabushiTwoFactorStatusResponse = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.auth.method("twoFactorStatus"),
            params: FabushiEmptyParams()
        ) {
            twoFactorStatus = status
        }
        if let rawList: [FabushiJSONValue] = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.auth.method("passkeyList"),
            params: FabushiEmptyParams()
        ) {
            passkeys = rawList.compactMap(FabushiPasskeyRecordDTO.fromJSONValue)
        }
    }

    private func saveLocalPasscode(useBiometrics: Bool = false) async {
        passcodeStatusError = nil
        guard newPasscodeInput.count >= FabushiPasscodeLockController.minimumPasscodeLength else {
            passcodeStatusError = theme.localized("hig.passcode.tooShortError")
            return
        }
        guard newPasscodeInput == confirmPasscodeInput else {
            passcodeStatusError = theme.localized("hig.passcode.mismatchError")
            return
        }

        let authorization: FabushiPasscodeReauthAuthorization?
        if passcodeController.isPasscodeEnabled {
            if useBiometrics {
                authorization = .biometrics(
                    localizedReason: theme.localized("hig.passcode.biometricChangeReason")
                )
            } else {
                guard !currentPasscodeInput.isEmpty else {
                    passcodeStatusError = theme.localized("hig.passcode.currentRequiredError")
                    return
                }
                authorization = .currentPasscode(currentPasscodeInput)
            }
        } else {
            authorization = nil
        }

        isWorking = true
        defer { isWorking = false }

        do {
            try await passcodeController.setPasscode(
                newPasscodeInput,
                currentAuthorization: authorization,
                coordinator: coordinator
            )
            router.isPasscodeLockEnabled = true
            currentPasscodeInput = ""
            newPasscodeInput = ""
            confirmPasscodeInput = ""
            showingPasscodeSheet = false
        } catch {
            currentPasscodeInput = ""
            passcodeStatusError = localizedPasscodeErrorMessage(for: error)
        }
    }

    private func removeLocalPasscode(useBiometrics: Bool = false) async {
        disablePasscodeError = nil
        let authorization: FabushiPasscodeReauthAuthorization
        if useBiometrics {
            authorization = .biometrics(
                localizedReason: theme.localized("hig.passcode.biometricDisableReason")
            )
        } else {
            guard !disablePasscodeInput.isEmpty else {
                disablePasscodeError = theme.localized("hig.passcode.currentRequiredError")
                return
            }
            authorization = .currentPasscode(disablePasscodeInput)
        }

        isWorking = true
        defer { isWorking = false }

        do {
            try await passcodeController.clearPasscode(
                authorization: authorization,
                coordinator: coordinator
            )
            router.isPasscodeLockEnabled = false
            router.isPasscodeLocked = false
            disablePasscodeInput = ""
            disablePasscodeError = nil
            showingPasscodeDisableSheet = false
        } catch {
            disablePasscodeInput = ""
            disablePasscodeError = localizedPasscodeErrorMessage(for: error)
        }
    }

    private func localizedPasscodeErrorMessage(for error: Error) -> String {
        guard let policyError = error as? FabushiPasscodePolicyError else {
            return error.localizedDescription
        }
        switch policyError {
        case .tooShort:
            return theme.localized("hig.passcode.tooShortError")
        case .confirmationMismatch:
            return theme.localized("hig.passcode.mismatchError")
        case .reauthenticationRequired:
            return theme.localized("hig.passcode.reauthRequiredError")
        case .currentPasscodeIncorrect(let attempts):
            return "\(theme.localized("hig.auth.passcodeIncorrect")) (\(attempts))"
        case .lockedOut(let remaining, let attempts):
            return "\(theme.localized("hig.passcode.lockoutActive")) \(remaining)s (\(attempts))"
        case .biometricVerificationFailed(let reason):
            let trimmed = reason.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty {
                return theme.localized("hig.passcode.biometricFailedError")
            }
            return "\(theme.localized("hig.passcode.biometricFailedError")): \(trimmed)"
        }
    }

    private func saveTwoFactorPassword() async {
        twoFactorError = nil
        isWorking = true
        defer { isWorking = false }

        do {
            let updated: FabushiTwoFactorStatusResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("twoFactorSet"),
                params: FabushiTwoFactorSetRequest(
                    currentPassword: twoFactorCurrentPassword.isEmpty ? nil : twoFactorCurrentPassword,
                    newPassword: twoFactorNewPassword,
                    hint: twoFactorHint.isEmpty ? nil : twoFactorHint,
                    recoveryEmail: twoFactorRecoveryEmail.isEmpty ? nil : twoFactorRecoveryEmail
                )
            )
            twoFactorStatus = updated
            showingTwoFactorSetSheet = false
        } catch {
            twoFactorError = error.localizedDescription
        }
    }

    private func disableTwoFactorPassword() async {
        twoFactorError = nil
        guard !twoFactorCurrentPassword.isEmpty else {
            twoFactorError = theme.localized("hig.auth.twoFactorMissingPassword")
            return
        }

        isWorking = true
        defer { isWorking = false }

        do {
            let _: FabushiTwoFactorDisableResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("twoFactorDisable"),
                params: FabushiTwoFactorDisableRequest(currentPassword: twoFactorCurrentPassword)
            )
            twoFactorStatus = FabushiTwoFactorStatusResponse(
                hasPassword: false,
                hint: nil,
                recoveryEmailPattern: nil
            )
            showingTwoFactorDisableSheet = false
        } catch {
            twoFactorError = error.localizedDescription
        }
    }

    private func registerNewPasskey() async {
        passkeyStatusMessage = nil
        isWorking = true
        defer { isWorking = false }

        let (userId, username): (UInt64, String)
        if case .authenticated(let uid, let uname, _) = coordinator.authState {
            userId = uid
            username = uname
        } else {
            return
        }

        do {
            let challenge: FabushiPasskeyChallengeDTO = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyChallenge"),
                params: FabushiPasskeyChallengeRequest(username: username)
            )
            let reg = try await passkeyController.performRegistration(
                rpId: challenge.rpId,
                challenge: challenge.challenge,
                username: username,
                userId: userId
            )
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyRegister"),
                params: FabushiPasskeyRegisterRequest(
                    credentialId: reg.credentialIdB64,
                    publicKey: reg.attestationObjectB64,
                    deviceName: "Fabushi iOS"
                )
            )
            passkeyStatusMessage = theme.localized("hig.security.passkeyRegistered")
            await loadSecurityStatus()
        } catch {
            passkeyStatusMessage = "\(theme.localized("hig.auth.passkeyUnavailable")) (\(error.localizedDescription))"
        }
    }

    private func deletePasskey(id: String) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("passkeyDelete"),
                params: FabushiPasskeyDeleteRequest(passkeyId: id)
            )
            passkeys.removeAll { $0.id == id }
        } catch {
            passkeyStatusMessage = error.localizedDescription
        }
    }

    private func acceptQrPairing(payload: String) async {
        guard let token = FabushiAuthValidation.extractQrPairToken(from: payload) else {
            qrPairStatusMessage = theme.localized("hig.security.qrPairInvalidPayload")
            return
        }
        isWorking = true
        defer { isWorking = false }

        do {
            let _: FabushiQrPairAcceptResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.auth.method("qrPairAccept"),
                params: FabushiQrPairAcceptRequest(pairToken: token)
            )
            qrPairStatusMessage = theme.localized("hig.security.qrPairAccepted")
        } catch {
            qrPairStatusMessage = error.localizedDescription
        }
    }
}

// MARK: - Active Device Sessions View (`fabushi.settings.devicesList`, F05)

@MainActor
public struct FabushiDevicesSettingsView: View {
    public let coordinator: FabushiCoordinator

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(ShellNavigationRouter.self) private var injectedRouter: ShellNavigationRouter?
    @State private var sessions: [FabushiActiveDeviceSessionDTO] = []
    @State private var isLoading: Bool = false
    @State private var isMutating: Bool = false
    @State private var errorMessage: String?

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private struct RevokeSessionParams: Encodable, Sendable {
        let sessionId: String

        enum CodingKeys: String, CodingKey {
            case sessionId = "session_id"
        }
    }

    private struct RevokeSessionResult: Decodable, Sendable {
        let revoked: Bool
    }

    private struct RevokeOtherResult: Decodable, Sendable {
        let revokedOthers: Bool

        enum CodingKeys: String, CodingKey {
            case revokedOthers = "revoked_others"
        }
    }

    public init(coordinator: FabushiCoordinator) {
        self.coordinator = coordinator
    }

    private var currentSession: FabushiActiveDeviceSessionDTO? {
        sessions.first(where: \.isCurrent)
    }

    private var otherSessions: [FabushiActiveDeviceSessionDTO] {
        sessions.filter { !$0.isCurrent }
    }

    public var body: some View {
        List {
            if let current = currentSession {
                Section(header: Text(theme.localized("hig.devices.thisDeviceHeader"))) {
                    deviceRow(session: current)
                }
            }

            Section(header: Text(theme.localized("hig.devices.activeSessionsHeader"))) {
                if isLoading && sessions.isEmpty {
                    ProgressView()
                        .frame(maxWidth: .infinity, alignment: .center)
                } else if otherSessions.isEmpty {
                    Text(theme.localized("hig.devices.noOtherDevices"))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(otherSessions) { session in
                        deviceRow(session: session)
                            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                Button(role: .destructive) {
                                    Task { await revokeSession(session.sessionId) }
                                } label: {
                                    Label(theme.localized("hig.devices.terminate"), systemImage: "xmark.circle")
                                }
                            }
                    }
                }
            }

            if let errorMessage {
                Section {
                    Text(errorMessage)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.destructiveColor)
                        .accessibilityLabel(errorMessage)
                }
            }

            Section {
                Button(role: .destructive) {
                    Task { await revokeAllOtherSessions() }
                } label: {
                    Label(
                        theme.localized("hig.devices.revokeAllOthers"),
                        systemImage: "hand.raised.fill"
                    )
                    .fabushiFont(.body)
                    .foregroundStyle(theme.destructiveColor)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(isMutating)
                .accessibilityLabel(theme.localized("hig.devices.revokeAllOthers"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.revokeOtherDevicesButton)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.settings.devices"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.devicesList)
        .refreshable {
            await loadSessions()
        }
        .task {
            await loadSessions()
        }
    }

    private func deviceRow(session: FabushiActiveDeviceSessionDTO) -> some View {
        HStack(spacing: 12) {
            Image(systemName: session.isCurrent ? "iphone.gen3" : "laptopcomputer")
                .fabushiFont(.title3)
                .foregroundStyle(session.isCurrent ? theme.accentColor : theme.textSecondary)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(session.deviceName)
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    if session.isCurrent {
                        Text(theme.localized("hig.devices.currentBadge"))
                            .fabushiFont(.caption)
                            .foregroundStyle(theme.accentForegroundColor)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(theme.accentColor, in: Capsule())
                    }
                }
                if let platform = session.platform, !platform.isEmpty {
                    Text(platform)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
                if let ip = session.ipAddress, !ip.isEmpty {
                    Text(ip)
                        .fabushiFont(.caption2)
                        .foregroundStyle(theme.textMuted)
                }
            }

            Spacer(minLength: 8)

            if !session.isCurrent {
                Button(role: .destructive) {
                    Task { await revokeSession(session.sessionId) }
                } label: {
                    Text(theme.localized("hig.devices.terminate"))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.destructiveColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(isMutating)
            }
        }
        .padding(.vertical, 4)
    }

    private func loadSessions() async {
        guard coordinator.isStarted else { return }
        errorMessage = nil
        isLoading = true
        defer { isLoading = false }

        do {
            sessions = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("list"),
                params: FabushiEmptyParams()
            )
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func revokeSession(_ sessionId: String) async {
        errorMessage = nil
        isMutating = true
        defer { isMutating = false }

        do {
            let _: RevokeSessionResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("revoke"),
                params: RevokeSessionParams(sessionId: sessionId)
            )
            sessions.removeAll { $0.sessionId == sessionId }
            injectedRouter?.toastController.showStagePill(theme.localized("hig.devices.terminatedToast"))
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func revokeAllOtherSessions() async {
        errorMessage = nil
        isMutating = true
        defer { isMutating = false }

        do {
            let _: RevokeOtherResult = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.session.method("revokeOther"),
                params: FabushiEmptyParams()
            )
            sessions.removeAll { !$0.isCurrent }
            injectedRouter?.toastController.showStagePill(theme.localized("hig.devices.revokedOthersToast"))
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
