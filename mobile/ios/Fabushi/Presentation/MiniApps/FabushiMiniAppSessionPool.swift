import CryptoKit
import Foundation
import WebKit

/// Retains live `FabushiFocusGatedWKWebView` instances keyed per `(accountScopeGeneration, appKey)`
/// so minimizing a Mini App to floating PiP and restoring from `floatingPiPPill` preserves live
/// DOM, WebMCP, and WASM state **in-place without reloading**, while strictly calling
/// `teardownSession()` and revoking `ActiveMiniAppWebMcpSessionRegistry.shared` on explicit close,
/// logout, or account switch (`D10-12` + `D12-01`).
@MainActor
public final class FabushiMiniAppSessionPool {
    public struct PoolKey: Hashable, Sendable {
        public let generation: UInt64
        public let appKey: String

        public init(generation: UInt64, appKey: String) {
            self.generation = generation
            self.appKey = appKey
        }
    }

    @MainActor
    public final class PooledSession {
        public let key: PoolKey
        public let webView: FabushiFocusGatedWKWebView
        public let navigationDelegate: FabushiFocusGatedNavigationDelegate
        public private(set) var loadedURL: URL?
        public private(set) var loadCount: Int = 0
        internal let bridgeSession: MiniAppWebMcpBridgeSession
        internal let messageReceiver: MiniAppPooledMessageReceiver

        internal init(
            key: PoolKey,
            webView: FabushiFocusGatedWKWebView,
            navigationDelegate: FabushiFocusGatedNavigationDelegate,
            bridgeSession: MiniAppWebMcpBridgeSession,
            messageReceiver: MiniAppPooledMessageReceiver
        ) {
            self.key = key
            self.webView = webView
            self.navigationDelegate = navigationDelegate
            self.bridgeSession = bridgeSession
            self.messageReceiver = messageReceiver
        }

        public func loadIfNeeded(_ url: URL) {
            guard loadedURL != url else { return }
            loadedURL = url
            loadCount += 1
            webView.load(URLRequest(url: url))
        }

        public func loadHardenedHTMLIfNeeded(_ html: String, baseURL: URL?) {
            let hardened = hardenGeneratedMiniAppDocument(html)
            loadCount += 1
            webView.loadHTMLString(hardened, baseURL: baseURL)
        }
    }

    @MainActor
    internal final class MiniAppPooledMessageReceiver: NSObject, WKScriptMessageHandler {
        private let nativeContextActions = MiniAppNativeContextActions()
        var onWebMcpMessage: (@MainActor ([String: Any]) -> Void)?

        func userContentController(
            _ userContentController: WKUserContentController,
            didReceive message: WKScriptMessage
        ) {
            guard let payload = message.body as? [String: Any] else { return }
            if message.name == miniAppNativeContextMenuHandler, let webView = message.webView {
                nativeContextActions.present(payload: payload, from: webView)
                return
            }
            onWebMcpMessage?(payload)
        }
    }

    private static var _sharedInstance: FabushiMiniAppSessionPool?
    public static var shared: FabushiMiniAppSessionPool {
        if let existing = _sharedInstance {
            return existing
        }
        let created = FabushiMiniAppSessionPool()
        _sharedInstance = created
        return created
    }

    private var sessions: [PoolKey: PooledSession] = [:]

    public init() {}

    public var activeCount: Int {
        sessions.count
    }

    public func existingSession(generation: UInt64, appKey: String) -> PooledSession? {
        sessions[PoolKey(generation: generation, appKey: appKey)]
    }

    public func acquireOrCreateSession(
        generation: UInt64,
        appKey: String,
        accountId: String?,
        onWebMcpMessage: (@MainActor ([String: Any]) -> Void)? = nil
    ) -> PooledSession {
        let key = PoolKey(generation: generation, appKey: appKey)
        if let existing = sessions[key], !existing.webView.didTeardownSession {
            if let onWebMcpMessage {
                existing.messageReceiver.onWebMcpMessage = onWebMcpMessage
            }
            return existing
        }

        let config = WKWebViewConfiguration()
        config.websiteDataStore = makeIsolatedDataStore(accountId: accountId, appKey: appKey)
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        config.userContentController.addUserScript(
            WKUserScript(
                source: miniAppNativeContextMenuJavaScript,
                injectionTime: .atDocumentEnd,
                forMainFrameOnly: true
            )
        )

        let webView = FabushiFocusGatedWKWebView(frame: .zero, configuration: config)
        webView.allowFirstResponder = false
        let navDelegate = FabushiFocusGatedNavigationDelegate()
        navDelegate.shouldAllowNavigation = { action in
            guard let url = action.request.url else { return false }
            if isTrustedWebMcpBridgeURL(url) {
                return true
            }
            guard let scheme = url.scheme?.lowercased(),
                  scheme == "https" || scheme == "http"
            else {
                return false
            }
            return url.host != nil
        }
        webView.navigationDelegate = navDelegate

        let receiver = MiniAppPooledMessageReceiver()
        receiver.onWebMcpMessage = onWebMcpMessage
        webView.registerWeakMessageHandler(receiver, name: miniAppNativeContextMenuHandler)
        webView.registerWeakMessageHandler(receiver, name: "fabushiWebMcp")

        let bridgeSession = MiniAppWebMcpBridgeSession(
            pluginInstanceId: "\(appKey):\(UUID().uuidString)",
            nonce: UUID().uuidString.replacingOccurrences(of: "-", with: ""),
            grants: [],
            generation: generation
        )

        let pooled = PooledSession(
            key: key,
            webView: webView,
            navigationDelegate: navDelegate,
            bridgeSession: bridgeSession,
            messageReceiver: receiver
        )
        sessions[key] = pooled
        return pooled
    }

    public func teardownSession(generation: UInt64, appKey: String, accountId: String?) {
        let key = PoolKey(generation: generation, appKey: appKey)
        if let pooled = sessions.removeValue(forKey: key) {
            pooled.webView.teardownSession()
            ActiveMiniAppWebMcpSessionRegistry.shared.unregister(
                pluginId: appKey,
                pluginInstanceId: pooled.bridgeSession.pluginInstanceId,
                generation: pooled.bridgeSession.generation
            )
        }
        if let accountId, !accountId.isEmpty {
            ActiveMiniAppWebMcpSessionRegistry.shared.revokeSession(
                accountKey: accountId,
                pluginId: appKey
            )
        }
    }

    public func teardownAll(revokingAccountId accountId: String? = nil) {
        let all = Array(sessions.values)
        sessions.removeAll()
        for pooled in all {
            pooled.webView.teardownSession()
            ActiveMiniAppWebMcpSessionRegistry.shared.unregister(
                pluginId: pooled.key.appKey,
                pluginInstanceId: pooled.bridgeSession.pluginInstanceId,
                generation: pooled.bridgeSession.generation
            )
        }
        if let accountId, !accountId.isEmpty {
            ActiveMiniAppWebMcpSessionRegistry.shared.revokeSessions(accountKey: accountId)
        }
    }

    private func makeIsolatedDataStore(accountId: String?, appKey: String) -> WKWebsiteDataStore {
        guard let accountId, !accountId.isEmpty else {
            return .nonPersistent()
        }
        let digest = SHA256.hash(data: Data("fabushi.miniapp.store.v1:\(accountId):\(appKey)".utf8))
        var bytes = Array(digest.prefix(16))
        bytes[6] = (bytes[6] & 0x0F) | 0x50
        bytes[8] = (bytes[8] & 0x3F) | 0x80
        let uuid = UUID(uuid: (
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
            bytes[8], bytes[9], bytes[10], bytes[11],
            bytes[12], bytes[13], bytes[14], bytes[15]
        ))
        return WKWebsiteDataStore(forIdentifier: uuid)
    }
}
