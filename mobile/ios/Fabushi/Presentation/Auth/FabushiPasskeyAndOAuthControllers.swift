@preconcurrency import AVFoundation
import AuthenticationServices
import Foundation
import os
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Native Apple WebAuthn / Passkey Controller (AuthenticationServices)

public enum FabushiWebAuthnError: LocalizedError, Sendable, Equatable {
    case ceremonyInProgress
    case invalidChallenge
    case cancelled
    case unexpectedCredential
    case notEntitledOrUnavailable(String)

    public var errorDescription: String? {
        switch self {
        case .ceremonyInProgress:
            return "A passkey request is already in progress."
        case .invalidChallenge:
            return "Invalid passkey challenge received from server."
        case .cancelled:
            return "Passkey sign-in was cancelled."
        case .unexpectedCredential:
            return "Unexpected credential returned by AuthenticationServices."
        case .notEntitledOrUnavailable(let detail):
            return detail
        }
    }
}

public struct FabushiWebAuthnAssertionResult: Sendable, Equatable {
    public let credentialIdB64: String
    public let clientDataJsonB64: String
    public let authenticatorDataB64: String
    public let signatureB64: String
}

public struct FabushiWebAuthnRegistrationResult: Sendable, Equatable {
    public let credentialIdB64: String
    public let attestationObjectB64: String
    public let clientDataJsonB64: String
}

@MainActor
public final class FabushiWebAuthnPasskeyController:
    NSObject,
    ASAuthorizationControllerDelegate,
    ASAuthorizationControllerPresentationContextProviding
{
    private enum PendingContinuation {
        case assertion(CheckedContinuation<FabushiWebAuthnAssertionResult, Error>)
        case registration(CheckedContinuation<FabushiWebAuthnRegistrationResult, Error>)
    }

    private var pending: PendingContinuation?
    private var activeController: ASAuthorizationController?

    public override init() {
        super.init()
    }

    nonisolated public static func decodeChallengeData(_ raw: String) -> Data? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if let standard = Data(base64Encoded: trimmed) {
            return standard
        }
        var base64URL = trimmed
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let remainder = base64URL.count % 4
        if remainder != 0 {
            base64URL += String(repeating: "=", count: 4 - remainder)
        }
        if let decoded = Data(base64Encoded: base64URL) {
            return decoded
        }
        return Data(trimmed.utf8)
    }

    public func performAssertion(
        rpId: String,
        challenge: String
    ) async throws -> FabushiWebAuthnAssertionResult {
        guard pending == nil else {
            throw FabushiWebAuthnError.ceremonyInProgress
        }
        let cleanRpId = rpId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanRpId.isEmpty, let challengeData = Self.decodeChallengeData(challenge) else {
            throw FabushiWebAuthnError.invalidChallenge
        }

        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: cleanRpId
        )
        let request = provider.createCredentialAssertionRequest(challenge: challengeData)
        request.userVerificationPreference = .preferred

        return try await withCheckedThrowingContinuation { continuation in
            self.pending = .assertion(continuation)
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            self.activeController = controller
            controller.performRequests()
        }
    }

    public func performRegistration(
        rpId: String,
        challenge: String,
        username: String,
        userId: UInt64
    ) async throws -> FabushiWebAuthnRegistrationResult {
        guard pending == nil else {
            throw FabushiWebAuthnError.ceremonyInProgress
        }
        let cleanRpId = rpId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleanRpId.isEmpty, let challengeData = Self.decodeChallengeData(challenge) else {
            throw FabushiWebAuthnError.invalidChallenge
        }

        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(
            relyingPartyIdentifier: cleanRpId
        )
        let userHandle = Data(String(userId).utf8)
        let request = provider.createCredentialRegistrationRequest(
            challenge: challengeData,
            name: username.isEmpty ? "user_\(userId)" : username,
            userID: userHandle
        )
        request.userVerificationPreference = .preferred

        return try await withCheckedThrowingContinuation { continuation in
            self.pending = .registration(continuation)
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            self.activeController = controller
            controller.performRequests()
        }
    }

    public func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithAuthorization authorization: ASAuthorization
    ) {
        guard controller === activeController, let current = pending else { return }
        pending = nil
        activeController = nil

        switch current {
        case .assertion(let continuation):
            guard let cred = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialAssertion else {
                continuation.resume(throwing: FabushiWebAuthnError.unexpectedCredential)
                return
            }
            continuation.resume(
                returning: FabushiWebAuthnAssertionResult(
                    credentialIdB64: cred.credentialID.base64EncodedString(),
                    clientDataJsonB64: cred.rawClientDataJSON.base64EncodedString(),
                    authenticatorDataB64: cred.rawAuthenticatorData.base64EncodedString(),
                    signatureB64: cred.signature.base64EncodedString()
                )
            )
        case .registration(let continuation):
            guard let cred = authorization.credential as? ASAuthorizationPlatformPublicKeyCredentialRegistration else {
                continuation.resume(throwing: FabushiWebAuthnError.unexpectedCredential)
                return
            }
            let attestation = cred.rawAttestationObject ?? Data()
            continuation.resume(
                returning: FabushiWebAuthnRegistrationResult(
                    credentialIdB64: cred.credentialID.base64EncodedString(),
                    attestationObjectB64: attestation.base64EncodedString(),
                    clientDataJsonB64: cred.rawClientDataJSON.base64EncodedString()
                )
            )
        }
    }

    public func authorizationController(
        controller: ASAuthorizationController,
        didCompleteWithError error: Error
    ) {
        guard controller === activeController, let current = pending else { return }
        pending = nil
        activeController = nil

        let mapped: Error
        if let authError = error as? ASAuthorizationError, authError.code == .canceled {
            mapped = FabushiWebAuthnError.cancelled
        } else {
            mapped = FabushiWebAuthnError.notEntitledOrUnavailable(error.localizedDescription)
        }

        switch current {
        case .assertion(let continuation):
            continuation.resume(throwing: mapped)
        case .registration(let continuation):
            continuation.resume(throwing: mapped)
        }
    }

    public func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        Self.resolvePresentationAnchor()
    }

    private static func resolvePresentationAnchor() -> ASPresentationAnchor {
        #if canImport(UIKit)
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        if let keyWindow = scenes.flatMap(\.windows).first(where: \.isKeyWindow) {
            return keyWindow
        }
        if let firstWindow = scenes.flatMap(\.windows).first {
            return firstWindow
        }
        #endif
        return ASPresentationAnchor()
    }
}

// MARK: - Native Apple ASWebAuthenticationSession Controller for Alipay Mobile OAuth (F03)

@MainActor
public final class FabushiAlipayWebAuthController:
    NSObject,
    ASWebAuthenticationPresentationContextProviding
{
    private var activeSession: ASWebAuthenticationSession?

    public override init() {
        super.init()
    }

    public func cancel() {
        activeSession?.cancel()
        activeSession = nil
    }

    public func startOAuthSession(
        url: URL,
        callbackScheme: String = "fabushi",
        onResult: @escaping @MainActor (Result<URL, Error>) -> Void
    ) {
        cancel()
        let session = ASWebAuthenticationSession(
            url: url,
            callbackURLScheme: callbackScheme
        ) { [weak self] callbackURL, error in
            Task { @MainActor in
                guard let self else { return }
                self.activeSession = nil
                if let callbackURL {
                    onResult(.success(callbackURL))
                } else if let error {
                    onResult(.failure(error))
                }
            }
        }
        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = false
        activeSession = session
        if !session.start() {
            activeSession = nil
            onResult(
                .failure(
                    NSError(
                        domain: "com.ombhrum.fabushi.auth.alipay",
                        code: -1,
                        userInfo: [NSLocalizedDescriptionKey: "Unable to launch Alipay authentication session."]
                    )
                )
            )
        }
    }

    public func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        #if canImport(UIKit)
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        if let keyWindow = scenes.flatMap(\.windows).first(where: \.isKeyWindow) {
            return keyWindow
        }
        if let firstWindow = scenes.flatMap(\.windows).first {
            return firstWindow
        }
        #endif
        return ASPresentationAnchor()
    }
}

// MARK: - Honest Camera Availability & QR Scanner Sheet (Device Camera + Simulator Manual Fallback)

public enum FabushiCameraAvailability: Sendable {
    public static var isHardwareCameraAvailable: Bool {
        #if targetEnvironment(simulator)
        return false
        #else
        return AVCaptureDevice.default(for: .video) != nil
        #endif
    }
}

private final class FabushiQRMetadataRelay: NSObject, AVCaptureMetadataOutputObjectsDelegate, Sendable {
    private let didEmit = OSAllocatedUnfairLock(initialState: false)
    private let onDetected: @Sendable (String) -> Void

    init(onDetected: @escaping @Sendable (String) -> Void) {
        self.onDetected = onDetected
    }

    func reset() {
        didEmit.withLock { $0 = false }
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        for object in metadataObjects {
            guard let readable = object as? AVMetadataMachineReadableCodeObject,
                  readable.type == .qr,
                  let payload = readable.stringValue?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !payload.isEmpty
            else {
                continue
            }
            let shouldEmit = didEmit.withLock { alreadyEmitted -> Bool in
                if alreadyEmitted { return false }
                alreadyEmitted = true
                return true
            }
            if shouldEmit {
                onDetected(payload)
            }
            return
        }
    }
}

#if canImport(UIKit)
@MainActor
private final class FabushiCameraPreviewViewController: UIViewController {
    private let captureSession = AVCaptureSession()
    private let metadataQueue = DispatchQueue(label: "com.ombhrum.fabushi.qr.metadata")
    private let sessionQueue = DispatchQueue(label: "com.ombhrum.fabushi.qr.session")
    private let relay: FabushiQRMetadataRelay
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private var isConfigured = false

    init(onCode: @escaping @Sendable (String) -> Void) {
        self.relay = FabushiQRMetadataRelay(onDetected: onCode)
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        configureCaptureSessionIfPossible()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        relay.reset()
        guard isConfigured else { return }
        let session = captureSession
        sessionQueue.async {
            if !session.isRunning {
                session.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        guard isConfigured else { return }
        let session = captureSession
        sessionQueue.async {
            if session.isRunning {
                session.stopRunning()
            }
        }
    }

    private func configureCaptureSessionIfPossible() {
        guard FabushiCameraAvailability.isHardwareCameraAvailable,
              let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              captureSession.canAddInput(input)
        else {
            return
        }

        captureSession.beginConfiguration()
        captureSession.addInput(input)

        let metadataOutput = AVCaptureMetadataOutput()
        if captureSession.canAddOutput(metadataOutput) {
            captureSession.addOutput(metadataOutput)
            metadataOutput.setMetadataObjectsDelegate(relay, queue: metadataQueue)
            if metadataOutput.availableMetadataObjectTypes.contains(.qr) {
                metadataOutput.metadataObjectTypes = [.qr]
            }
        }
        captureSession.commitConfiguration()

        let layer = AVCaptureVideoPreviewLayer(session: captureSession)
        layer.videoGravity = .resizeAspectFill
        layer.frame = view.bounds
        view.layer.addSublayer(layer)
        previewLayer = layer
        isConfigured = true
    }
}

@MainActor
private struct FabushiCameraPreviewRepresentable: UIViewControllerRepresentable {
    let onDetectedCode: @MainActor (String) -> Void

    func makeUIViewController(context: Context) -> FabushiCameraPreviewViewController {
        FabushiCameraPreviewViewController { payload in
            Task { @MainActor in
                onDetectedCode(payload)
            }
        }
    }

    func updateUIViewController(_ uiViewController: FabushiCameraPreviewViewController, context: Context) {}
}
#endif

/// Reusable QR Scanner Sheet for Cross-Device QR Pairing (`auth.qrPairAccept`) and TOTP `otpauth://` import.
/// Honestly detects Simulator / unavailable camera hardware and provides manual payload entry fallback.
@MainActor
public struct FabushiQRScannerSheet: View {
    public let titleKey: String
    public let manualPlaceholderKey: String
    public let onScannedPayload: (String) -> Void
    public let onCancel: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var manualPayload: String = ""
    @State private var cameraAuthStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)

    private var theme: FabushiTheme { injectedTheme ?? .default }

    public init(
        titleKey: String,
        manualPlaceholderKey: String,
        onScannedPayload: @escaping (String) -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.titleKey = titleKey
        self.manualPlaceholderKey = manualPlaceholderKey
        self.onScannedPayload = onScannedPayload
        self.onCancel = onCancel
    }

    public var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    #if canImport(UIKit)
                    if FabushiCameraAvailability.isHardwareCameraAvailable && cameraAuthStatus == .authorized {
                        FabushiCameraPreviewRepresentable { payload in
                            onScannedPayload(payload)
                        }
                        .frame(height: 260)
                        .clipShape(RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, style: .continuous)
                                .stroke(theme.accentColor.opacity(0.6), lineWidth: 2)
                        )
                    } else {
                        VStack(spacing: 10) {
                            Image(systemName: "qrcode.viewfinder")
                                .fabushiFont(.largeTitle)
                                .foregroundStyle(theme.accentColor)
                                .accessibilityHidden(true)

                            Text(theme.localized("hig.auth.qrScannerCameraUnavailable"))
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textSecondary)
                                .multilineTextAlignment(.center)

                            if FabushiCameraAvailability.isHardwareCameraAvailable && cameraAuthStatus == .notDetermined {
                                FabushiGlassPillButton(
                                    title: theme.localized("hig.auth.qrScannerGrantCamera"),
                                    systemImage: "camera.fill",
                                    accessibilityID: "fabushi.internal.qrScanner.grantCamera",
                                    isProminent: false
                                ) {
                                    Task {
                                        let granted = await AVCaptureDevice.requestAccess(for: .video)
                                        cameraAuthStatus = granted ? .authorized : .denied
                                    }
                                }
                            }
                        }
                        .padding(20)
                        .frame(maxWidth: .infinity)
                        .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.modalCornerRadius, isInteractive: false)
                    }
                    #endif

                    VStack(alignment: .leading, spacing: 10) {
                        Text(theme.localized("hig.auth.qrScannerManualTitle"))
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.textPrimary)

                        TextField(
                            theme.localized(manualPlaceholderKey),
                            text: $manualPayload,
                            axis: .vertical
                        )
                        .fabushiFont(.body)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .lineLimit(2...4)
                        .padding(12)
                        .background(theme.surfaceInput, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .accessibilityLabel(theme.localized(manualPlaceholderKey))

                        FabushiGlassPillButton(
                            title: theme.localized("hig.common.apply"),
                            systemImage: "checkmark.circle.fill",
                            accessibilityID: "fabushi.internal.qrScanner.applyManual",
                            isProminent: true
                        ) {
                            let trimmed = manualPayload.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !trimmed.isEmpty else { return }
                            onScannedPayload(trimmed)
                        }
                        .disabled(manualPayload.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                    .padding(16)
                    .fabushiLiquidGlass(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, isInteractive: false)
                }
                .padding(20)
            }
            .background(theme.surfacePrimary)
            .navigationTitle(theme.localized(titleKey))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(theme.localized("hig.common.cancel")) {
                        onCancel()
                    }
                    .fabushiFont(.body)
                    .fabushiMinimumTouchTarget()
                }
            }
        }
    }
}
