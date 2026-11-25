import AuthenticationServices
import Foundation
#if canImport(UIKit)
import UIKit
#endif

public enum FabushiMagpieOAuthError: LocalizedError, Sendable, Equatable {
    case invalidAuthorizeURL
    case unableToStartSession
    case userCancelled
    case providerError(String)

    public var errorDescription: String? {
        switch self {
        case .invalidAuthorizeURL:
            return "Invalid HTTPS OAuth authorization URL."
        case .unableToStartSession:
            return "Unable to launch browser authentication session."
        case .userCancelled:
            return "OAuth sign-in was cancelled."
        case .providerError(let detail):
            return detail
        }
    }
}

public struct FabushiMagpieOAuthCallbackPayload: Sendable, Equatable {
    public let callbackURL: URL
    public let code: String?
    public let state: String?
    public let error: String?

    public init(callbackURL: URL) {
        self.callbackURL = callbackURL
        let components = URLComponents(url: callbackURL, resolvingAgainstBaseURL: false)
        let queryItems = components?.queryItems ?? []
        self.code = queryItems.first(where: { $0.name == "code" })?.value
        self.state = queryItems.first(where: { $0.name == "state" })?.value
        self.error = queryItems.first(where: { $0.name == "error" })?.value
    }
}

/// Native iOS `ASWebAuthenticationSession` controller for Magpie AI Provider OAuth sign-in
/// using the registered `"fabushi"` callback URL scheme.
///
/// Cancellation-safe and Swift 6 strict-concurrency clean: tracks active session ownership
/// via an identity token so stale completion handlers never overwrite a newer session.
@MainActor
public final class FabushiMagpieOAuthController:
    NSObject,
    ASWebAuthenticationPresentationContextProviding
{
    public static let defaultCallbackScheme = "fabushi"

    private var activeSession: ASWebAuthenticationSession?
    private var activeSessionId: UUID?

    public var isAuthenticating: Bool {
        activeSession != nil
    }

    public override init() {
        super.init()
    }

    public func cancel() {
        activeSessionId = nil
        activeSession?.cancel()
        activeSession = nil
    }

    /// Validates that an authorization URL uses `https://` (or loopback `http://127.0.0.1`/`localhost` for tests).
    public static func validateAuthorizeURL(_ rawURL: String) -> URL? {
        let trimmed = rawURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              let host = components.host?.lowercased(),
              !host.isEmpty
        else {
            return nil
        }
        if scheme == "https" {
            return components.url
        }
        if scheme == "http" && (host == "127.0.0.1" || host == "localhost" || host == "::1") {
            return components.url
        }
        return nil
    }

    public func startOAuthSession(
        url: URL,
        callbackScheme: String = FabushiMagpieOAuthController.defaultCallbackScheme,
        prefersEphemeralSession: Bool = false,
        onResult: @escaping @MainActor (Result<FabushiMagpieOAuthCallbackPayload, FabushiMagpieOAuthError>) -> Void
    ) {
        cancel()

        let sessionId = UUID()
        let session = ASWebAuthenticationSession(
            url: url,
            callbackURLScheme: callbackScheme
        ) { [weak self] callbackURL, error in
            let capturedError = error
            let capturedURL = callbackURL
            Task { @MainActor in
                guard let self, self.activeSessionId == sessionId else {
                    return
                }
                self.activeSessionId = nil
                self.activeSession = nil

                if let capturedURL {
                    let payload = FabushiMagpieOAuthCallbackPayload(callbackURL: capturedURL)
                    if let oauthErr = payload.error, !oauthErr.isEmpty {
                        onResult(.failure(.providerError(oauthErr)))
                    } else {
                        onResult(.success(payload))
                    }
                    return
                }

                if let authErr = capturedError as? ASWebAuthenticationSessionError,
                   authErr.code == .canceledLogin
                {
                    onResult(.failure(.userCancelled))
                } else if let capturedError {
                    onResult(.failure(.providerError(capturedError.localizedDescription)))
                } else {
                    onResult(.failure(.userCancelled))
                }
            }
        }

        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = prefersEphemeralSession
        activeSessionId = sessionId
        activeSession = session

        if !session.start() {
            if activeSessionId == sessionId {
                activeSessionId = nil
                activeSession = nil
            }
            onResult(.failure(.unableToStartSession))
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
