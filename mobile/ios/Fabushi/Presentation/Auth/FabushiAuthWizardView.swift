import SwiftUI

/// Unified Apple HIG Authentication Wizard covering F01–F05 (`fabushi.auth.root`).
///
/// Supports both `.primaryGate` (cold launch) and `.addAccount` (from Multi-Account Switcher
/// while preserving existing active accounts). Exposes all canonical `FabushiAccessibilityIDs.Auth.*`
/// identifiers directly in a single scrollable hierarchy with Dynamic Type scaling up to AX5,
/// `>= 44×44pt` touch targets, and Liquid Glass surfaces.
@MainActor
public struct FabushiAuthWizardView: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let launchConfiguration: FabushiLaunchConfiguration
    public let mode: FabushiAuthPresentationMode
    public let onCancelAddAccount: (() -> Void)?
    public let onAuthenticated: (() -> Void)?

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var viewModel: FabushiAuthViewModel

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        launchConfiguration: FabushiLaunchConfiguration = .current,
        mode: FabushiAuthPresentationMode = .primaryGate,
        onCancelAddAccount: (() -> Void)? = nil,
        onAuthenticated: (() -> Void)? = nil
    ) {
        self.coordinator = coordinator
        self.router = router
        self.launchConfiguration = launchConfiguration
        self.mode = mode
        self.onCancelAddAccount = onCancelAddAccount
        self.onAuthenticated = onAuthenticated
        self._viewModel = State(
            initialValue: FabushiAuthViewModel(
                coordinator: coordinator,
                router: router,
                launchConfiguration: launchConfiguration,
                mode: mode,
                onAuthenticated: onAuthenticated
            )
        )
    }

    public var body: some View {
        Group {
            if mode == .primaryGate {
                NavigationStack {
                    scrollContent
                }
            } else {
                scrollContent
                    .navigationTitle(theme.localized("hig.accountSwitcher.addAccount"))
                    .navigationBarTitleDisplayMode(.inline)
                    .navigationBarBackButtonHidden(
                        onCancelAddAccount != nil
                            || viewModel.isAwaitingEmergencyKitBackup
                            || viewModel.isSubmitting
                    )
                    .toolbar {
                        if let onCancelAddAccount, !viewModel.isAwaitingEmergencyKitBackup {
                            ToolbarItem(placement: .topBarLeading) {
                                Button(theme.localized("hig.common.cancel")) {
                                    viewModel.cancelActiveTasks()
                                    onCancelAddAccount()
                                }
                                .fabushiFont(.body)
                                .fabushiMinimumTouchTarget()
                                .disabled(viewModel.isSubmitting)
                            }
                        }
                    }
            }
        }
        .interactiveDismissDisabled(viewModel.isAwaitingEmergencyKitBackup || viewModel.isSubmitting)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.root)
        .onChange(of: router.lastAuthCallbackURL) { _, newURL in
            guard newURL != nil, let callbackURL = router.consumeLastAuthCallbackURL() else { return }
            Task {
                await viewModel.handleAuthCallbackURL(callbackURL, theme: theme)
            }
        }
        .onAppear {
            viewModel.handleViewAppear()
            if let callbackURL = router.consumeLastAuthCallbackURL() {
                Task {
                    await viewModel.handleAuthCallbackURL(callbackURL, theme: theme)
                }
            }
        }
        .onDisappear {
            viewModel.handleViewDisappear()
        }
    }

    private var scrollContent: some View {
        ScrollView {
            VStack(spacing: 18) {
                headerSection
                if !viewModel.isAwaitingEmergencyKitBackup {
                    serverConfigurationCard
                }
                if let emergencyKit = viewModel.generatedEmergencyKit, !emergencyKit.isEmpty {
                    emergencyKitBackupCard(emergencyKit: emergencyKit)
                } else if let challenge = viewModel.activeTwoFactorChallenge {
                    twoFactorGateCard(challenge: challenge)
                } else {
                    primaryCredentialsCard
                    otpVerificationCard
                    alternativeSignInCard
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 20)
            .frame(maxWidth: 480)
            .frame(maxWidth: .infinity)
        }
        .background(theme.surfacePrimary)
    }

    // MARK: - Header

    private var headerSection: some View {
        VStack(spacing: 8) {
            Image(systemName: "paperplane.circle.fill")
                .fabushiFont(.largeTitle)
                .foregroundStyle(theme.accentColor)
                .accessibilityHidden(true)

            Text(
                mode == .addAccount
                    ? theme.localized("hig.accountSwitcher.addAccount")
                    : theme.localized("hig.auth.title")
            )
            .fabushiFont(.title2)
            .foregroundStyle(theme.textPrimary)
            .multilineTextAlignment(.center)

            Text(theme.localized("hig.auth.subtitle"))
                .fabushiFont(.subheadline)
                .foregroundStyle(theme.textSecondary)
                .multilineTextAlignment(.center)
        }
        .padding(.top, mode == .primaryGate ? 8 : 0)
    }

    // MARK: - Server URL Card (4-Tier Config Cascade + Fail-Closed Inline Validation)

    private var serverConfigurationCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            FabushiSectionHeader(theme.localized("hig.auth.serverSectionTitle"))

            TextField(
                theme.localized("hig.auth.serverURLPlaceholder"),
                text: $viewModel.serverURLText
            )
            .fabushiFont(.body)
            .keyboardType(.URL)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .padding(12)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityLabel(theme.localized("hig.auth.serverURLPlaceholder"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.serverURLField)

            if let serverURLError = viewModel.serverURLError {
                Text(serverURLError)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.destructiveColor)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityLabel(serverURLError)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.serverURLError)
            }
        }
        .padding(16)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
    }

    // MARK: - F01: Sign In / Register (Split-Key V2)

    private var primaryCredentialsCard: some View {
        VStack(alignment: .leading, spacing: 14) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    signInTabButton
                    registerTabButton
                }
                VStack(spacing: 8) {
                    signInTabButton
                    registerTabButton
                }
            }

            if viewModel.authTab == .register {
                HStack(spacing: 8) {
                    Image(systemName: "lock.shield.fill")
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.successColor)
                        .accessibilityHidden(true)
                    Text(theme.localized("hig.auth.splitKeyBadgeLabel"))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textPrimary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    theme.surfaceInput,
                    in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                )
                .accessibilityElement(children: .combine)
                .accessibilityLabel(theme.localized("hig.auth.splitKeyBadgeLabel"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.splitKeyBadge)
            }

            TextField(
                theme.localized("hig.auth.usernamePlaceholder"),
                text: $viewModel.usernameText
            )
            .fabushiFont(.body)
            .textContentType(.username)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .padding(12)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityLabel(theme.localized("hig.auth.usernamePlaceholder"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.usernameField)

            SecureField(
                theme.localized("hig.auth.passwordPlaceholder"),
                text: $viewModel.passwordText
            )
            .fabushiFont(.body)
            .textContentType(viewModel.authTab == .register ? .newPassword : .password)
            .padding(12)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityLabel(theme.localized("hig.auth.passwordPlaceholder"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.passwordField)

            if viewModel.authTab == .register {
                TextField(
                    theme.localized("hig.auth.firstNamePlaceholder"),
                    text: $viewModel.firstNameText
                )
                .fabushiFont(.body)
                .textContentType(.givenName)
                .padding(12)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityLabel(theme.localized("hig.auth.firstNamePlaceholder"))
            }

            if let errorMessage = viewModel.errorMessage {
                Text(errorMessage)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.destructiveColor)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityLabel(errorMessage)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.errorBanner)
            }

            if viewModel.authTab == .signIn {
                FabushiGlassPillButton(
                    title: theme.localized("hig.auth.loginButton"),
                    systemImage: "arrow.right.circle.fill",
                    accessibilityID: FabushiAccessibilityIDs.Auth.loginButton,
                    isProminent: true
                ) {
                    Task { await viewModel.submitLogin(theme: theme) }
                }
                .frame(maxWidth: .infinity)
                .disabled(viewModel.isSubmitting)
            } else {
                FabushiGlassPillButton(
                    title: theme.localized("hig.auth.registerSubmitButton"),
                    systemImage: "person.badge.key.fill",
                    accessibilityID: FabushiAccessibilityIDs.Auth.registerSubmitButton,
                    isProminent: true
                ) {
                    Task { await viewModel.submitRegistration(theme: theme) }
                }
                .frame(maxWidth: .infinity)
                .disabled(viewModel.isSubmitting)
            }
        }
        .padding(16)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
    }

    private var signInTabButton: some View {
        Button {
            viewModel.authTab = .signIn
            viewModel.clearErrors()
        } label: {
            Text(theme.localized("hig.auth.loginButton"))
                .fabushiFont(.callout)
                .fontWeight(.semibold)
                .foregroundStyle(viewModel.authTab == .signIn ? theme.accentForegroundColor : theme.textPrimary)
                .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
                .background(
                    viewModel.authTab == .signIn ? theme.accentColor : theme.surfaceInput,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                )
        }
        .buttonStyle(.plain)
        .accessibilityLabel(theme.localized("hig.auth.loginButton"))
    }

    private var registerTabButton: some View {
        Button {
            viewModel.authTab = .register
            viewModel.clearErrors()
        } label: {
            Text(theme.localized("hig.auth.registerTab"))
                .fabushiFont(.callout)
                .fontWeight(.semibold)
                .foregroundStyle(viewModel.authTab == .register ? theme.accentForegroundColor : theme.textPrimary)
                .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
                .background(
                    viewModel.authTab == .register ? theme.accentColor : theme.surfaceInput,
                    in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.pillButtonCornerRadius, style: .continuous)
                )
        }
        .buttonStyle(.plain)
        .accessibilityLabel(theme.localized("hig.auth.registerTab"))
        .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.registerTabButton)
    }

    // MARK: - F02: Quick Phone SMS & Email OTP Card (Directly Reachable on Initial Screen)

    private var otpVerificationCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            FabushiSectionHeader(theme.localized("hig.auth.otpSectionTitle"))

            Picker(
                theme.localized("hig.auth.otpSectionTitle"),
                selection: $viewModel.otpChannel
            ) {
                Text(theme.localized("hig.auth.otpPhoneTab")).tag(FabushiOTPChannel.phone)
                Text(theme.localized("hig.auth.otpEmailTab")).tag(FabushiOTPChannel.email)
            }
            .pickerStyle(.segmented)

            if viewModel.otpChannel == .phone {
                HStack(spacing: 8) {
                    Menu {
                        ForEach(FabushiCountryDialCode.commonCodes) { item in
                            Button("\(item.regionCode) \(item.dialCode)") {
                                viewModel.selectedDialCode = item
                            }
                        }
                    } label: {
                        Text("\(viewModel.selectedDialCode.regionCode) \(viewModel.selectedDialCode.dialCode)")
                            .fabushiFont(.callout)
                            .foregroundStyle(theme.textPrimary)
                            .padding(.horizontal, 10)
                            .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
                            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                    .accessibilityLabel(theme.localized("hig.auth.countryCodePicker"))

                    TextField(
                        theme.localized("hig.auth.phonePlaceholder"),
                        text: $viewModel.phoneText
                    )
                    .fabushiFont(.body)
                    .keyboardType(.phonePad)
                    .textContentType(.telephoneNumber)
                    .padding(12)
                    .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .accessibilityLabel(theme.localized("hig.auth.phonePlaceholder"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.phoneField)
                }
            } else {
                TextField(
                    theme.localized("hig.auth.emailPlaceholder"),
                    text: $viewModel.emailText
                )
                .fabushiFont(.body)
                .keyboardType(.emailAddress)
                .textContentType(.emailAddress)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(12)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityLabel(theme.localized("hig.auth.emailPlaceholder"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.emailField)
            }

            let sendTitle = viewModel.otpResendRemainingSeconds > 0
                ? "\(theme.localized("hig.auth.resendOTP")) (\(viewModel.otpResendRemainingSeconds)s)"
                : theme.localized("hig.auth.sendOTPButton")

            FabushiGlassPillButton(
                title: sendTitle,
                systemImage: "paperplane.fill",
                accessibilityID: FabushiAccessibilityIDs.Auth.sendOTPButton,
                isProminent: false
            ) {
                Task { await viewModel.sendOTP(theme: theme) }
            }
            .frame(maxWidth: .infinity)
            .disabled(viewModel.isSubmitting || viewModel.otpResendRemainingSeconds > 0)

            if let otpStatusMessage = viewModel.otpStatusMessage {
                Text(otpStatusMessage)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.successColor)
            }

            if viewModel.otpCodeSent {
                TextField(
                    theme.localized("hig.auth.otpCodePlaceholder"),
                    text: $viewModel.otpCodeText
                )
                .fabushiFont(.body)
                .keyboardType(.numberPad)
                .textContentType(.oneTimeCode)
                .padding(12)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityLabel(theme.localized("hig.auth.otpCodePlaceholder"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.otpCodeField)

                FabushiGlassPillButton(
                    title: theme.localized("hig.auth.verifyOTPButton"),
                    systemImage: "checkmark.shield.fill",
                    accessibilityID: FabushiAccessibilityIDs.Auth.verifyOTPButton,
                    isProminent: true
                ) {
                    Task { await viewModel.verifyOTP(theme: theme) }
                }
                .frame(maxWidth: .infinity)
                .disabled(viewModel.isSubmitting)
            }
        }
        .padding(16)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
    }

    // MARK: - F03 & F04: Alipay QR / OAuth, Passkeys & QR Device Pairing

    private var alternativeSignInCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            FabushiSectionHeader(theme.localized("hig.auth.moreMethodsTitle"))

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.alipayLoginButton"),
                systemImage: "qrcode",
                accessibilityID: FabushiAccessibilityIDs.Auth.alipayLoginButton,
                isProminent: false
            ) {
                Task { await viewModel.startAlipayLogin(theme: theme) }
            }
            .frame(maxWidth: .infinity)
            .disabled(viewModel.isSubmitting)

            if viewModel.isAlipayActive {
                alipayExpandedSection
            }

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.passkeyLoginButton"),
                systemImage: "person.badge.key.fill",
                accessibilityID: FabushiAccessibilityIDs.Auth.passkeyLoginButton,
                isProminent: false
            ) {
                Task { await viewModel.signInWithPasskey(theme: theme) }
            }
            .frame(maxWidth: .infinity)
            .disabled(viewModel.isSubmitting)

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.qrPairButton"),
                systemImage: "laptopcomputer.and.iphone",
                accessibilityID: FabushiAccessibilityIDs.Auth.qrPairButton,
                isProminent: false
            ) {
                Task { await viewModel.startQrPair(theme: theme) }
            }
            .frame(maxWidth: .infinity)
            .disabled(viewModel.isSubmitting)

            if viewModel.isQrPairActive {
                qrPairExpandedSection
            }
        }
        .padding(16)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
    }

    private var alipayExpandedSection: some View {
        VStack(spacing: 12) {
            Group {
                #if canImport(UIKit)
                if let qrImage = viewModel.alipayQRImage {
                    Image(uiImage: qrImage)
                        .interpolation(.none)
                        .resizable()
                        .scaledToFit()
                        .frame(width: 180, height: 180)
                        .padding(12)
                        .background(Color.white, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                } else {
                    ProgressView()
                        .frame(width: 180, height: 180)
                }
                #endif
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(theme.localized("hig.auth.alipayQRCodeLabel"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.alipayQRView)

            if let statusText = viewModel.alipayStatusText {
                Text(statusText)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .multilineTextAlignment(.center)
                    .accessibilityLabel(statusText)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.alipayStatusLabel)
            }

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    alipayOpenAppButton
                    alipayCancelButton
                }
                VStack(spacing: 8) {
                    alipayOpenAppButton
                    alipayCancelButton
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity)
        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    private var alipayOpenAppButton: some View {
        Button {
            viewModel.launchAlipayMobileOAuth(theme: theme)
        } label: {
            Text(theme.localized("hig.auth.alipayOpenOAuth"))
                .fabushiFont(.footnote)
                .fontWeight(.semibold)
                .foregroundStyle(theme.accentColor)
                .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
        }
        .buttonStyle(.plain)
    }

    private var alipayCancelButton: some View {
        Button {
            viewModel.cancelAlipayLogin()
        } label: {
            Text(theme.localized("hig.common.cancel"))
                .fabushiFont(.footnote)
                .foregroundStyle(theme.textSecondary)
                .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
        }
        .buttonStyle(.plain)
    }

    private var qrPairExpandedSection: some View {
        VStack(spacing: 12) {
            Group {
                #if canImport(UIKit)
                if let qrImage = viewModel.qrPairImage {
                    Image(uiImage: qrImage)
                        .interpolation(.none)
                        .resizable()
                        .scaledToFit()
                        .frame(width: 180, height: 180)
                        .padding(12)
                        .background(Color.white, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                } else {
                    ProgressView()
                        .frame(width: 180, height: 180)
                }
                #endif
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(theme.localized("hig.auth.qrPairCanvasLabel"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.qrPairCanvas)

            if let statusText = viewModel.qrPairStatusText {
                Text(statusText)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .multilineTextAlignment(.center)
            }

            Button {
                viewModel.cancelQrPair()
            } label: {
                Text(theme.localized("hig.common.cancel"))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textSecondary)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
            }
            .buttonStyle(.plain)
        }
        .padding(14)
        .frame(maxWidth: .infinity)
        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    // MARK: - F05: Cloud 2FA Verification Gate

    private func twoFactorGateCard(challenge: FabushiTwoFactorChallenge) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Image(systemName: "key.viewfinder")
                    .fabushiFont(.title3)
                    .foregroundStyle(theme.accentColor)
                    .accessibilityHidden(true)
                Text(theme.localized("hig.auth.twoFactorTitle"))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
            }

            Text(theme.localized("hig.auth.twoFactorSubtitle"))
                .fabushiFont(.footnote)
                .foregroundStyle(theme.textSecondary)

            if let hint = challenge.hint, !hint.isEmpty {
                Text("\(theme.localized("hig.auth.twoFactorHintPrefix")) \(hint)")
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textPrimary)
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }

            SecureField(
                theme.localized("hig.auth.twoFactorPlaceholder"),
                text: $viewModel.twoFactorPasswordText
            )
            .fabushiFont(.body)
            .textContentType(.password)
            .padding(12)
            .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityLabel(theme.localized("hig.auth.twoFactorPlaceholder"))
            .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.twoFactorPasswordField)

            if let errorMessage = viewModel.errorMessage {
                Text(errorMessage)
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.destructiveColor)
                    .accessibilityLabel(errorMessage)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Auth.errorBanner)
            }

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.twoFactorVerifyButton"),
                systemImage: "checkmark.shield.fill",
                accessibilityID: FabushiAccessibilityIDs.Auth.twoFactorVerifyButton,
                isProminent: true
            ) {
                Task { await viewModel.submitTwoFactorVerification(theme: theme) }
            }
            .frame(maxWidth: .infinity)
            .disabled(viewModel.isSubmitting)

            Button {
                viewModel.cancelTwoFactorChallenge()
            } label: {
                Text(theme.localized("hig.common.back"))
                    .fabushiFont(.callout)
                    .foregroundStyle(theme.textSecondary)
                    .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget)
            }
            .buttonStyle(.plain)
        }
        .padding(16)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
    }

    // MARK: - D4: One-Time Emergency Kit Backup Card (Split-Key V2 Registration)

    private func emergencyKitBackupCard(emergencyKit: String) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 8) {
                Image(systemName: "key.viewfinder")
                    .fabushiFont(.title3)
                    .foregroundStyle(theme.warningColor)
                    .accessibilityHidden(true)
                Text(theme.localized("hig.auth.emergencyKitTitle"))
                    .fabushiFont(.headline)
                    .foregroundStyle(theme.textPrimary)
            }

            Text(theme.localized("hig.auth.emergencyKitSubtitle"))
                .fabushiFont(.footnote)
                .foregroundStyle(theme.textSecondary)

            Text(emergencyKit)
                .fabushiFont(.monospacedCaption)
                .foregroundStyle(theme.textPrimary)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .accessibilityLabel(emergencyKit)
                .accessibilityIdentifier("fabushi.internal.auth.emergencyKitValue")

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.emergencyKitCopyButton"),
                systemImage: "doc.on.doc",
                accessibilityID: "fabushi.internal.auth.emergencyKitCopyButton",
                isProminent: false
            ) {
                _ = viewModel.copyGeneratedEmergencyKitToSecurePasteboard()
            }
            .frame(maxWidth: .infinity)

            if viewModel.didCopyEmergencyKit {
                Text(theme.localized("hig.auth.emergencyKitCopiedNotice"))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.successColor)
            }

            Toggle(isOn: $viewModel.hasAcknowledgedEmergencyKitSaved) {
                Text(theme.localized("hig.auth.emergencyKitAckLabel"))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textPrimary)
            }
            .frame(minHeight: FabushiLayoutMetrics.minTouchTarget)
            .accessibilityIdentifier("fabushi.internal.auth.emergencyKitAckToggle")

            FabushiGlassPillButton(
                title: theme.localized("hig.auth.emergencyKitContinueButton"),
                systemImage: "checkmark.shield.fill",
                accessibilityID: "fabushi.internal.auth.emergencyKitContinueButton",
                isProminent: true
            ) {
                _ = viewModel.acknowledgeEmergencyKitAndContinue()
            }
            .frame(maxWidth: .infinity)
            .disabled(!viewModel.hasAcknowledgedEmergencyKitSaved)
        }
        .padding(16)
        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("fabushi.internal.auth.emergencyKitCard")
    }
}
