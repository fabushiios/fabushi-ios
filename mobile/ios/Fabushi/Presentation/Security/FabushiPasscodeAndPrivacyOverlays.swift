import SwiftUI

// MARK: - Full-Screen Passcode Lock Overlay (`fabushi.auth.passcodeOverlay`)

@MainActor
public struct FabushiPasscodeLockOverlayView: View {
    public let coordinator: FabushiCoordinator
    public let launchConfiguration: FabushiLaunchConfiguration
    public let passcodeController: FabushiPasscodeLockController
    public let onUnlock: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var passcodeInput: String = ""
    @State private var errorMessage: String?
    @State private var isVerifying: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        launchConfiguration: FabushiLaunchConfiguration,
        passcodeController: FabushiPasscodeLockController = .shared,
        onUnlock: @escaping () -> Void
    ) {
        self.coordinator = coordinator
        self.launchConfiguration = launchConfiguration
        self.passcodeController = passcodeController
        self.onUnlock = onUnlock
    }

    public var body: some View {
        let biometry = passcodeController.canEvaluateBiometrics()

        ScrollView {
            VStack(spacing: 20) {
                Spacer(minLength: 40)

                Image(systemName: "lock.shield.fill")
                    .fabushiFont(.largeTitle)
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)

                VStack(spacing: 6) {
                    Text(theme.localized("hig.auth.passcodeTitle"))
                        .fabushiFont(.title2)
                        .foregroundStyle(theme.textPrimary)
                        .multilineTextAlignment(.center)

                    Text(theme.localized("hig.passcode.overlaySubtitle"))
                        .fabushiFont(.subheadline)
                        .foregroundStyle(theme.textSecondary)
                        .multilineTextAlignment(.center)
                }

                VStack(spacing: 14) {
                    SecureField(
                        theme.localized("hig.auth.passcodePlaceholder"),
                        text: $passcodeInput
                    )
                    .fabushiFont(.body)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .onSubmit {
                        if !isVerifying {
                            Task { await submitPasscode() }
                        }
                    }
                    .padding(12)
                    .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .accessibilityLabel(theme.localized("hig.auth.passcodePlaceholder"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passcodeField)

                    if let errorMessage {
                        Text(errorMessage)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityLabel(errorMessage)
                            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passcodeErrorBanner)
                    }

                    FabushiGlassPillButton(
                        title: theme.localized("hig.auth.passcodeUnlock"),
                        systemImage: "lock.open.fill",
                        accessibilityID: FabushiAccessibilityIDs.Auth.passcodeSubmitButton,
                        isProminent: true
                    ) {
                        Task { await submitPasscode() }
                    }
                    .frame(maxWidth: .infinity)
                    .disabled(isVerifying)

                    if passcodeController.biometricsEnabled && biometry.available {
                        FabushiGlassPillButton(
                            title: theme.localized(biometry.kind.localizationKey),
                            systemImage: biometry.kind.systemImage,
                            accessibilityID: "fabushi.internal.passcode.biometricsButton",
                            isProminent: false
                        ) {
                            Task { await unlockWithBiometrics() }
                        }
                        .frame(maxWidth: .infinity)
                        .disabled(isVerifying)
                    }
                }
                .padding(20)
                .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, isInteractive: false)

                Spacer(minLength: 40)
            }
            .padding(24)
            .frame(maxWidth: 420)
            .frame(maxWidth: .infinity)
        }
        .background(theme.surfaceModal.ignoresSafeArea())
        .accessibilityElement(children: .contain)
        .accessibilityLabel(theme.localized("hig.auth.passcodeTitle"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passcodeOverlay)
    }

    private func submitPasscode() async {
        errorMessage = nil
        isVerifying = true
        defer { isVerifying = false }

        let result = await passcodeController.verifyPasscode(passcodeInput, coordinator: coordinator)
        switch result {
        case .unlocked:
            passcodeInput = ""
            errorMessage = nil
            onUnlock()
        case .incorrect(let attempts):
            passcodeInput = ""
            errorMessage = "\(theme.localized("hig.auth.passcodeIncorrect")) (\(attempts))"
        case .lockedOut(let remainingSeconds, let attempts):
            passcodeInput = ""
            errorMessage = "\(theme.localized("hig.passcode.lockoutActive")) \(remainingSeconds)s (\(attempts))"
        }
    }

    private func unlockWithBiometrics() async {
        errorMessage = nil
        isVerifying = true
        defer { isVerifying = false }

        let result = await passcodeController.authenticateWithBiometrics(
            localizedReason: theme.localized("hig.passcode.biometricReason")
        )
        switch result {
        case .success:
            passcodeInput = ""
            onUnlock()
        case .failure(let error):
            if case .cancelled = error {
                errorMessage = theme.localized("hig.passcode.biometricCancelled")
            } else {
                errorMessage = error.localizedDescription
            }
        }
    }
}

// MARK: - Full-Screen Background Privacy Blur Shield (`fabushi.auth.privacyBlurOverlay`)

@MainActor
public struct FabushiPrivacyBlurOverlayView: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init() {}

    public var body: some View {
        ZStack {
            if reduceTransparency {
                theme.surfaceModal.ignoresSafeArea()
            } else {
                Rectangle()
                    .fill(.ultraThickMaterial)
                    .ignoresSafeArea()
            }

            VStack(spacing: 14) {
                Image(systemName: "lock.shield.fill")
                    .fabushiFont(.largeTitle)
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)

                Text(theme.localized("hig.app.name"))
                    .fabushiFont(.title2)
                    .foregroundStyle(theme.textPrimary)

                Text(theme.localized("hig.auth.privacyProtected"))
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textSecondary)
                    .multilineTextAlignment(.center)
            }
            .padding(24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(theme.localized("hig.auth.privacyProtected"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.privacyBlurOverlay)
    }
}
