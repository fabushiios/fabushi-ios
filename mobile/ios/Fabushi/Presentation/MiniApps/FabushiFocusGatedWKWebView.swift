import Foundation
import UIKit
import WebKit

// MARK: - D12-01 Deterministic WKWebView Focus Retention Policy

/// Snapshot of host UI presentation and occlusion state used to determine whether an embedded
/// `WKWebView` (Mini App or Built-in Browser) is allowed to claim `UIResponder`/keyboard/IME focus.
public struct FabushiWebViewFocusContext: Sendable, Equatable {
    public let isPresented: Bool
    public let isMinimizedToPiP: Bool
    public let isBackgroundWebMcp: Bool
    public let hasBlockingModal: Bool
    public let isHostInputFocused: Bool

    public init(
        isPresented: Bool,
        isMinimizedToPiP: Bool = false,
        isBackgroundWebMcp: Bool = false,
        hasBlockingModal: Bool = false,
        isHostInputFocused: Bool = false
    ) {
        self.isPresented = isPresented
        self.isMinimizedToPiP = isMinimizedToPiP
        self.isBackgroundWebMcp = isBackgroundWebMcp
        self.hasBlockingModal = hasBlockingModal
        self.isHostInputFocused = isHostInputFocused
    }
}

/// Pure deterministic predicate implementing upstream `v1.6.12` `D12-01`:
/// `WKWebView` may own keyboard/IME focus ONLY when it is actively presented, unoccluded,
/// not minimized to PiP, not running as a background WebMCP surface, and no native host text
/// field (composer, URL bar, search field, modal field) currently owns focus.
public enum FabushiWebViewFocusPolicy: Sendable {
    public static func shouldWebViewOwnKeyboard(_ context: FabushiWebViewFocusContext) -> Bool {
        context.isPresented
            && !context.isMinimizedToPiP
            && !context.isBackgroundWebMcp
            && !context.hasBlockingModal
            && !context.isHostInputFocused
    }
}

// MARK: - Weak WKScriptMessageHandler Proxy (Retain-Cycle Prevention)

/// Non-retaining `WKScriptMessageHandler` trampoline preventing strong retain cycles between
/// `WKUserContentController` and host ViewModels/delegates.
@MainActor
public final class FabushiWeakScriptMessageHandlerProxy: NSObject, WKScriptMessageHandler {
    private weak var target: (any WKScriptMessageHandler)?

    public init(target: any WKScriptMessageHandler) {
        self.target = target
        super.init()
    }

    public func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        target?.userContentController(userContentController, didReceive: message)
    }
}

// MARK: - D12-01 Focus-Gated WKWebView Subclass

/// Focus-gated `WKWebView` shared by Mini Apps (`Presentation/MiniApps/`) and the Built-in
/// Interactive Browser (`Presentation/Browser/`).
///
/// Guarantees:
/// - `allowFirstResponder` defaults to `false` so parked, minimized PiP, occluded, or background
///   WebMCP instances can NEVER hijack the iOS software keyboard or IME.
/// - `passThroughTouchesWhenNonInteractive` routes touches to enclosing SwiftUI PiP controls
///   instead of the web surface.
/// - `blurActiveDOMElement()` blurs `document.activeElement` and resigns first responder whenever
///   a native host input activates or a modal occludes the viewport.
/// - `teardownSession()` cleanly stops navigation, resigns first responder, removes all registered
///   `WKScriptMessageHandler`s, and clears delegates on close, logout, or account switch.
@MainActor
public final class FabushiFocusGatedWKWebView: WKWebView {
    public static let blurActiveElementJS = """
    (function(){
      try {
        if (document && document.activeElement && typeof document.activeElement.blur === 'function') {
          document.activeElement.blur();
        }
      } catch (_) {}
    })();
    """

    public var allowFirstResponder: Bool = false {
        didSet {
            if !allowFirstResponder {
                _ = resignFirstResponder()
            }
        }
    }

    public var passThroughTouchesWhenNonInteractive: Bool = false

    public private(set) var domBlurEvaluationCount: Int = 0
    public private(set) var didTeardownSession: Bool = false
    public private(set) var registeredMessageHandlerNames: Set<String> = []

    public override init(frame: CGRect, configuration: WKWebViewConfiguration) {
        super.init(frame: frame, configuration: configuration)
        allowsBackForwardNavigationGestures = true
    }

    @available(*, unavailable)
    public required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported for FabushiFocusGatedWKWebView")
    }

    public override var canBecomeFirstResponder: Bool {
        guard allowFirstResponder else { return false }
        return super.canBecomeFirstResponder
    }

    @discardableResult
    public override func becomeFirstResponder() -> Bool {
        guard allowFirstResponder else { return false }
        return super.becomeFirstResponder()
    }

    public override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        if passThroughTouchesWhenNonInteractive && !allowFirstResponder {
            return nil
        }
        return super.hitTest(point, with: event)
    }

    public func registerWeakMessageHandler(
        _ handler: any WKScriptMessageHandler,
        name: String
    ) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if registeredMessageHandlerNames.contains(trimmed) {
            configuration.userContentController.removeScriptMessageHandler(forName: trimmed)
        }
        let proxy = FabushiWeakScriptMessageHandlerProxy(target: handler)
        configuration.userContentController.add(proxy, name: trimmed)
        registeredMessageHandlerNames.insert(trimmed)
    }

    public func unregisterAllMessageHandlers() {
        for name in registeredMessageHandlerNames {
            configuration.userContentController.removeScriptMessageHandler(forName: name)
        }
        registeredMessageHandlerNames.removeAll()
    }

    public func blurActiveDOMElement() {
        domBlurEvaluationCount += 1
        _ = resignFirstResponder()
        endEditing(true)
        evaluateJavaScript(Self.blurActiveElementJS, completionHandler: nil)
    }

    public func enforceFocusPolicy(allowFirstResponder allowed: Bool) {
        let wasAllowed = allowFirstResponder
        allowFirstResponder = allowed
        if !allowed {
            if wasAllowed || isFirstResponder {
                blurActiveDOMElement()
            } else {
                _ = resignFirstResponder()
                endEditing(true)
            }
        }
    }

    @discardableResult
    public func enforceFocusPolicy(context: FabushiWebViewFocusContext) -> Bool {
        let allowed = FabushiWebViewFocusPolicy.shouldWebViewOwnKeyboard(context)
        passThroughTouchesWhenNonInteractive = context.isMinimizedToPiP || context.isBackgroundWebMcp
        enforceFocusPolicy(allowFirstResponder: allowed)
        return allowed
    }

    public func teardownSession() {
        guard !didTeardownSession else { return }
        didTeardownSession = true
        allowFirstResponder = false
        passThroughTouchesWhenNonInteractive = true
        stopLoading()
        blurActiveDOMElement()
        unregisterAllMessageHandlers()
        configuration.userContentController.removeAllUserScripts()
        navigationDelegate = nil
        uiDelegate = nil
        loadHTMLString("", baseURL: nil)
    }
}

// MARK: - Focus-Gated Navigation Delegate (Blocks <input autofocus> Focus Theft)

@MainActor
public class FabushiFocusGatedNavigationDelegate: NSObject, WKNavigationDelegate {
    public var shouldAllowNavigation: (@MainActor (WKNavigationAction) -> Bool)?
    public var onDidStartNavigation: (@MainActor (WKWebView) -> Void)?
    public var onDidFinishNavigation: (@MainActor (WKWebView) -> Void)?
    public var onDidFailNavigation: (@MainActor (WKWebView, Error) -> Void)?

    public override init() {
        super.init()
    }

    public func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction
    ) async -> WKNavigationActionPolicy {
        if let shouldAllowNavigation {
            return shouldAllowNavigation(navigationAction) ? .allow : .cancel
        }
        return .allow
    }

    public func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        onDidStartNavigation?(webView)
    }

    public func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if let focusWebView = webView as? FabushiFocusGatedWKWebView,
           !focusWebView.allowFirstResponder
        {
            focusWebView.blurActiveDOMElement()
        }
        onDidFinishNavigation?(webView)
    }

    public func webView(
        _ webView: WKWebView,
        didFail navigation: WKNavigation!,
        withError error: any Error
    ) {
        onDidFailNavigation?(webView, error)
    }

    public func webView(
        _ webView: WKWebView,
        didFailProvisionalNavigation navigation: WKNavigation!,
        withError error: any Error
    ) {
        onDidFailNavigation?(webView, error)
    }
}
