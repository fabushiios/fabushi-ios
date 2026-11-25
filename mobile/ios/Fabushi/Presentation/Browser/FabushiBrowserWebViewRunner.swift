import Foundation
import os
import UIKit
import WebKit

// MARK: - WK_DOM_RECORDER_JS & Semantic Locator Cascade Scripts (`D11-06`)

public enum FabushiBrowserScripts: Sendable {
    public static let recorderHandlerName = "fabushiBrowserRecorder"

    /// Injected into `WKUserContentController` to capture interactive user actions
    /// (`click`, `type`, `press_key`, `scroll`, `navigate`) with deterministic semantic locators
    /// and automatic sensitive-field redaction (`password`, `otp`, `totp`, `token`, `secret`, `cvv`,
    /// credit-card / one-time-code `autocomplete`).
    public static let wkDomRecorderJS: String = """
    (function() {
      if (window.__fabushiDomRecorderInstalled) return;
      window.__fabushiDomRecorderInstalled = true;
      window.__fabushiRecordingEnabled = false;

      const SENSITIVE_RE = /(password|passwd|pwd|otp|totp|one-time-code|token|secret|cvv|cvc|cc-number|cc-csc|pin)/i;
      const SENSITIVE_AUTOCOMPLETE = new Set([
        'current-password', 'new-password', 'one-time-code',
        'cc-number', 'cc-csc', 'cc-exp'
      ]);

      function isSensitiveElement(el) {
        if (!el) return false;
        const type = String(el.getAttribute && el.getAttribute('type') || '').toLowerCase();
        if (type === 'password') return true;
        const ac = String(el.getAttribute && el.getAttribute('autocomplete') || '').toLowerCase().trim();
        if (SENSITIVE_AUTOCOMPLETE.has(ac)) return true;
        const probe = [
          el.getAttribute && el.getAttribute('name') || '',
          el.getAttribute && el.getAttribute('id') || '',
          el.getAttribute && el.getAttribute('placeholder') || '',
          el.getAttribute && el.getAttribute('aria-label') || ''
        ].join(' ');
        return SENSITIVE_RE.test(probe);
      }

      function buildCssSelector(el) {
        if (!el || !el.tagName) return '';
        if (el.id) return '#' + CSS.escape(el.id);
        let selector = el.tagName.toLowerCase();
        const nameAttr = el.getAttribute && el.getAttribute('name');
        if (nameAttr) {
          selector += '[name="' + nameAttr.replace(/"/g, '\\\\"') + '"]';
          return selector;
        }
        const cls = (el.className && typeof el.className === 'string')
          ? el.className.trim().split(/\\s+/).filter(Boolean).slice(0, 2)
          : [];
        if (cls.length) selector += '.' + cls.map(CSS.escape).join('.');
        return selector;
      }

      function buildLocator(el) {
        if (!el) return {};
        const testId = el.getAttribute && el.getAttribute('data-testid');
        const ariaLabel = el.getAttribute && el.getAttribute('aria-label');
        const role = (el.getAttribute && el.getAttribute('role')) || (el.tagName ? el.tagName.toLowerCase() : null);
        const textAnchor = (el.innerText || el.textContent || '').trim().slice(0, 80) || null;
        const nameAttr = el.getAttribute && el.getAttribute('name');
        const cssSelector = buildCssSelector(el);
        return {
          test_id: testId || null,
          aria_label: ariaLabel || null,
          role: role || null,
          text_anchor: textAnchor,
          name_attr: nameAttr || null,
          css_selector: cssSelector || null
        };
      }

      function post(payload) {
        if (!window.__fabushiRecordingEnabled) return;
        try {
          window.webkit.messageHandlers.fabushiBrowserRecorder.postMessage(payload);
        } catch (_) {}
      }

      document.addEventListener('click', function(ev) {
        const el = ev.target instanceof Element ? ev.target.closest('a,button,input,textarea,select,[role],[data-testid]') || ev.target : null;
        if (!el) return;
        post({
          event_type: 'click',
          page_url: window.location.href,
          locator: buildLocator(el)
        });
      }, true);

      document.addEventListener('change', function(ev) {
        const el = ev.target instanceof Element ? ev.target : null;
        if (!el) return;
        const sensitive = isSensitiveElement(el);
        const rawVal = typeof el.value === 'string' ? el.value : '';
        const paramHint = (el.getAttribute && (el.getAttribute('name') || el.getAttribute('id'))) || null;
        post({
          event_type: 'type',
          page_url: window.location.href,
          locator: buildLocator(el),
          value: sensitive ? '[REDACTED]' : rawVal,
          is_secret: sensitive,
          parameter_name_hint: paramHint
        });
      }, true);

      document.addEventListener('keydown', function(ev) {
        if (ev.key === 'Enter' || ev.key === 'Escape' || ev.key === 'Tab') {
          post({
            event_type: 'press_key',
            page_url: window.location.href,
            key: ev.key
          });
        }
      }, true);

      let scrollTimer = null;
      window.addEventListener('scroll', function() {
        if (!window.__fabushiRecordingEnabled) return;
        clearTimeout(scrollTimer);
        scrollTimer = setTimeout(function() {
          post({
            event_type: 'scroll',
            page_url: window.location.href,
            delta_x: window.scrollX || 0,
            delta_y: window.scrollY || 0
          });
        }, 250);
      }, { passive: true });
    })();
    """

    public static func setRecordingEnabledJS(_ enabled: Bool) -> String {
        "window.__fabushiRecordingEnabled = \(enabled ? "true" : "false");"
    }

    /// Generates JavaScript resolving a `SemanticLocator` using the strict deterministic priority
    /// cascade: `1. test_id (data-testid) -> 2. aria_label -> 3. role + text_anchor -> 4. name_attr -> 5. css`.
    public static func semanticLocatorOperationJS(
        locator: FabushiBrowserSemanticLocator,
        operation: String,
        value: String? = nil
    ) -> String {
        let payloadDict: [String: Any] = [
            "test_id": locator.testId ?? NSNull(),
            "aria_label": locator.ariaLabel ?? NSNull(),
            "role": locator.role ?? NSNull(),
            "text_anchor": locator.textAnchor ?? NSNull(),
            "name_attr": locator.nameAttr ?? NSNull(),
            "css_selector": locator.cssSelector ?? NSNull(),
            "operation": operation,
            "value": value ?? NSNull(),
        ]
        let jsonData = (try? JSONSerialization.data(withJSONObject: payloadDict, options: [])) ?? Data("{}".utf8)
        let jsonLiteral = String(decoding: jsonData, as: UTF8.self)

        return """
        (function(cfg) {
          function findElement(c) {
            if (c.test_id) {
              const el = document.querySelector('[data-testid="' + String(c.test_id).replace(/"/g, '\\\\"') + '"]');
              if (el) return { el, strategy: 'test_id' };
            }
            if (c.aria_label) {
              const el = document.querySelector('[aria-label="' + String(c.aria_label).replace(/"/g, '\\\\"') + '"]');
              if (el) return { el, strategy: 'aria_label' };
            }
            if (c.role && c.text_anchor) {
              const wantRole = String(c.role).toLowerCase();
              const wantText = String(c.text_anchor).trim().toLowerCase();
              const candidates = Array.from(document.querySelectorAll('[role="' + wantRole + '"],' + wantRole));
              for (const node of candidates) {
                const text = (node.innerText || node.textContent || '').trim().toLowerCase();
                if (text.includes(wantText)) return { el: node, strategy: 'role+text_anchor' };
              }
            }
            if (c.name_attr) {
              const el = document.querySelector('[name="' + String(c.name_attr).replace(/"/g, '\\\\"') + '"]');
              if (el) return { el, strategy: 'name_attr' };
            }
            if (c.css_selector) {
              try {
                const el = document.querySelector(c.css_selector);
                if (el) return { el, strategy: 'css' };
              } catch (_) {}
            }
            return null;
          }

          const found = findElement(cfg);
          if (!found) return JSON.stringify({ ok: false, error: 'Element not found' });
          const el = found.el;
          if (cfg.operation === 'click') {
            el.click();
            return JSON.stringify({ ok: true, strategy: found.strategy });
          }
          if (cfg.operation === 'fill') {
            el.focus && el.focus();
            el.value = String(cfg.value || '');
            el.dispatchEvent(new Event('input', { bubbles: true }));
            el.dispatchEvent(new Event('change', { bubbles: true }));
            return JSON.stringify({ ok: true, strategy: found.strategy });
          }
          if (cfg.operation === 'extract_text') {
            return JSON.stringify({ ok: true, strategy: found.strategy, text: (el.innerText || el.textContent || '').trim() });
          }
          if (cfg.operation === 'assert_visible') {
            const rect = el.getBoundingClientRect();
            const visible = rect.width > 0 || rect.height > 0;
            return JSON.stringify({ ok: visible, strategy: found.strategy });
          }
          if (cfg.operation === 'assert_text_contains') {
            const actual = (el.innerText || el.textContent || '');
            const expected = String(cfg.value || '');
            return JSON.stringify({ ok: actual.includes(expected), strategy: found.strategy, text: actual });
          }
          return JSON.stringify({ ok: true, strategy: found.strategy });
        })(\(jsonLiteral));
        """
    }
}

// MARK: - Apple-Native Interactive Browser WKWebView Runner (`D11-06` + `D12-01`)

public enum FabushiBrowserRunnerError: LocalizedError, Sendable, Equatable {
    case unsafeURL(String)
    case elementNotFound(String)
    case scriptExecutionFailed(String)
    case missingRequiredParameter(String)
    case humanCheckpointPaused(String)

    public var errorDescription: String? {
        switch self {
        case .unsafeURL(let raw):
            return "Blocked unsafe browser navigation URL: \(raw)"
        case .elementNotFound(let desc):
            return "Locator did not match any DOM element: \(desc)"
        case .scriptExecutionFailed(let msg):
            return "Browser script step failed: \(msg)"
        case .missingRequiredParameter(let name):
            return "Missing required parameter '\(name)'"
        case .humanCheckpointPaused(let reason):
            return "Paused for human checkpoint: \(reason)"
        }
    }
}

@MainActor
public final class FabushiBrowserWebViewRunner: NSObject, WKScriptMessageHandler {
    public private(set) var activeProfile: FabushiBrowserProfileRecord
    public private(set) var webView: FabushiFocusGatedWKWebView
    public private(set) var navigationDelegate: FabushiFocusGatedNavigationDelegate
    public private(set) var isRecording: Bool = false

    public var onRecordedAction: (@MainActor (String?, FabushiRecordedActionKind) -> Void)?
    public var onNavigationChanged: (@MainActor (URL?, String?, Bool, Bool, Bool) -> Void)?
    public var onNavigationBlocked: (@MainActor (String) -> Void)?

    private let nativeContextActions = MiniAppNativeContextActions()
    private var mirroredCookies: [FabushiBrowserCookieRecord] = []

    public init(profile: FabushiBrowserProfileRecord) {
        self.activeProfile = profile
        let built = Self.buildWebView(for: profile)
        self.webView = built.webView
        self.navigationDelegate = built.navDelegate
        super.init()
        configureDelegatesAndScripts()
    }

    private static func buildWebView(
        for profile: FabushiBrowserProfileRecord
    ) -> (webView: FabushiFocusGatedWKWebView, navDelegate: FabushiFocusGatedNavigationDelegate) {
        let config = WKWebViewConfiguration()
        if profile.isEphemeral {
            config.websiteDataStore = .nonPersistent()
        } else if let uuid = profile.storeIdentifier {
            config.websiteDataStore = WKWebsiteDataStore(forIdentifier: uuid)
        } else {
            config.websiteDataStore = .nonPersistent()
        }
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        config.userContentController.addUserScript(
            WKUserScript(
                source: FabushiBrowserScripts.wkDomRecorderJS,
                injectionTime: .atDocumentEnd,
                forMainFrameOnly: false
            )
        )
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
        webView.navigationDelegate = navDelegate
        return (webView, navDelegate)
    }

    private func configureDelegatesAndScripts() {
        webView.registerWeakMessageHandler(self, name: FabushiBrowserScripts.recorderHandlerName)
        webView.registerWeakMessageHandler(self, name: miniAppNativeContextMenuHandler)

        navigationDelegate.shouldAllowNavigation = { [weak self] action in
            guard let url = action.request.url else { return false }
            let raw = url.absoluteString
            if raw == "about:blank" {
                return true
            }
            if FabushiBrowserSecurity.validateWKNavigationURL(raw) != nil {
                return true
            }
            self?.onNavigationBlocked?(raw)
            return false
        }

        navigationDelegate.onDidStartNavigation = { [weak self] wv in
            self?.onNavigationChanged?(wv.url, wv.title, wv.canGoBack, wv.canGoForward, true)
        }

        navigationDelegate.onDidFinishNavigation = { [weak self] wv in
            guard let self else { return }
            self.onNavigationChanged?(wv.url, wv.title, wv.canGoBack, wv.canGoForward, false)
            if self.isRecording {
                wv.evaluateJavaScript(FabushiBrowserScripts.setRecordingEnabledJS(true), completionHandler: nil)
                if let urlStr = wv.url?.absoluteString,
                   FabushiBrowserSecurity.validateWKNavigationURL(urlStr) != nil
                {
                    self.onRecordedAction?(urlStr, .navigate(url: urlStr))
                }
            }
        }

        navigationDelegate.onDidFailNavigation = { [weak self] wv, _ in
            self?.onNavigationChanged?(wv.url, wv.title, wv.canGoBack, wv.canGoForward, false)
        }
    }

    /// Switches to an isolated `WKWebsiteDataStore(forIdentifier:)` (or `.nonPersistent()`)
    /// after cleanly tearing down the previous WebView session.
    public func switchProfile(_ profile: FabushiBrowserProfileRecord) {
        let wasAllowed = webView.allowFirstResponder
        webView.teardownSession()
        activeProfile = profile
        let built = Self.buildWebView(for: profile)
        webView = built.webView
        navigationDelegate = built.navDelegate
        configureDelegatesAndScripts()
        webView.enforceFocusPolicy(allowFirstResponder: wasAllowed)
    }

    public func navigate(to rawURL: String) throws {
        guard let url = FabushiBrowserSecurity.validateWKNavigationURL(rawURL) else {
            onNavigationBlocked?(rawURL)
            throw FabushiBrowserRunnerError.unsafeURL(rawURL)
        }
        webView.load(URLRequest(url: url))
    }

    public func goBack() {
        if webView.canGoBack {
            webView.goBack()
        }
    }

    public func goForward() {
        if webView.canGoForward {
            webView.goForward()
        }
    }

    public func reload() {
        webView.reload()
    }

    public func stopLoading() {
        webView.stopLoading()
    }

    public func setRecording(_ enabled: Bool) {
        isRecording = enabled
        webView.evaluateJavaScript(FabushiBrowserScripts.setRecordingEnabledJS(enabled), completionHandler: nil)
    }

    public func blurActiveDOMElement() {
        webView.blurActiveDOMElement()
    }

    public func enforceFocusPolicy(context: FabushiWebViewFocusContext) {
        _ = webView.enforceFocusPolicy(context: context)
    }

    public func teardownSession() {
        isRecording = false
        webView.teardownSession()
    }

    // MARK: - WKHTTPCookieStore Export / Import / Clear & PNG Snapshot

    public func exportCookies() async -> [FabushiBrowserCookieRecord] {
        let store = webView.configuration.websiteDataStore.httpCookieStore
        let live: [FabushiBrowserCookieRecord]? = await Self.runWithBoundedTimeout(seconds: 1.0) { complete in
            store.getAllCookies { cookies in
                let mapped = cookies.map { cookie in
                    FabushiBrowserCookieRecord(
                        name: cookie.name,
                        value: cookie.value,
                        domain: cookie.domain,
                        path: cookie.path,
                        expiresUnixSeconds: cookie.expiresDate?.timeIntervalSince1970,
                        secure: cookie.isSecure,
                        httpOnly: cookie.isHTTPOnly
                    )
                }
                complete(mapped)
            }
        }
        if let live, !live.isEmpty {
            mirroredCookies = live
            return live
        }
        return mirroredCookies
    }

    public func importCookies(_ records: [FabushiBrowserCookieRecord]) async -> Int {
        let store = webView.configuration.websiteDataStore.httpCookieStore
        var imported = 0
        for record in records {
            var props: [HTTPCookiePropertyKey: Any] = [
                .name: record.name,
                .value: record.value,
                .domain: record.domain,
                .path: record.path.isEmpty ? "/" : record.path,
            ]
            if let expires = record.expiresUnixSeconds, expires > 0 {
                props[.expires] = Date(timeIntervalSince1970: expires)
            }
            if record.secure {
                props[.secure] = "TRUE"
            }
            if let cookie = HTTPCookie(properties: props) {
                let _: Bool? = await Self.runWithBoundedTimeout(seconds: 0.75) { complete in
                    store.setCookie(cookie) {
                        complete(true)
                    }
                }
                imported += 1
            }
        }
        for record in records {
            if let idx = mirroredCookies.firstIndex(where: { $0.id == record.id }) {
                mirroredCookies[idx] = record
            } else {
                mirroredCookies.append(record)
            }
        }
        return imported
    }

    public func clearWebsiteData() async {
        mirroredCookies.removeAll()
        let dataStore = webView.configuration.websiteDataStore
        let allTypes = WKWebsiteDataStore.allWebsiteDataTypes()
        let _: Bool? = await Self.runWithBoundedTimeout(seconds: 1.0) { complete in
            dataStore.removeData(ofTypes: allTypes, modifiedSince: .distantPast) {
                complete(true)
            }
        }
    }

    public func captureSnapshotPNG() async -> Data? {
        let config = WKSnapshotConfiguration()
        let image = try? await webView.takeSnapshot(configuration: config)
        return image?.pngData()
    }

    private static func runWithBoundedTimeout<T: Sendable>(
        seconds: TimeInterval,
        operation: @escaping @MainActor (@escaping @Sendable (T) -> Void) -> Void
    ) async -> T? {
        await withCheckedContinuation { continuation in
            let resumed = OSAllocatedUnfairLock(initialState: false)
            let finish: @Sendable (T?) -> Void = { value in
                let shouldResume = resumed.withLock { already -> Bool in
                    if already { return false }
                    already = true
                    return true
                }
                if shouldResume {
                    continuation.resume(returning: value)
                }
            }
            operation { value in
                finish(value)
            }
            Task { @MainActor in
                let nanos = UInt64(max(0.05, seconds) * 1_000_000_000)
                try? await Task.sleep(nanoseconds: nanos)
                finish(nil)
            }
        }
    }

    // MARK: - WKScriptMessageHandler (DOM Recorder + Native Context Menu)

    public func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard let payload = message.body as? [String: Any] else { return }
        if message.name == miniAppNativeContextMenuHandler {
            nativeContextActions.present(payload: payload, from: webView)
            return
        }
        guard message.name == FabushiBrowserScripts.recorderHandlerName, isRecording else {
            return
        }
        let pageURL = payload["page_url"] as? String
        if let action = Self.parseRecorderPayload(payload) {
            onRecordedAction?(pageURL, action)
        }
    }

    /// Pure parser translating `WK_DOM_RECORDER_JS` payloads into `FabushiRecordedActionKind`
    /// while enforcing defense-in-depth sensitive field redaction on the Swift side as well.
    public static func parseRecorderPayload(_ payload: [String: Any]) -> FabushiRecordedActionKind? {
        let eventType = (payload["event_type"] as? String ?? "").lowercased()
        let locatorDict = payload["locator"] as? [String: Any] ?? [:]
        let locator = FabushiBrowserSemanticLocator(
            testId: locatorDict["test_id"] as? String,
            ariaLabel: locatorDict["aria_label"] as? String,
            role: locatorDict["role"] as? String,
            textAnchor: locatorDict["text_anchor"] as? String,
            cssSelector: locatorDict["css_selector"] as? String,
            nameAttr: locatorDict["name_attr"] as? String
        )

        switch eventType {
        case "navigate":
            guard let url = payload["url"] as? String ?? payload["page_url"] as? String,
                  FabushiBrowserSecurity.validateWKNavigationURL(url) != nil
            else { return nil }
            return .navigate(url: url)
        case "click":
            guard !locator.isEmpty else { return nil }
            return .click(locator: locator)
        case "type", "fill":
            guard !locator.isEmpty else { return nil }
            let hint = payload["parameter_name_hint"] as? String
            let jsFlaggedSecret = (payload["is_secret"] as? Bool) ?? false
            let locatorHint = [locator.nameAttr, locator.ariaLabel, locator.testId, hint]
                .compactMap { $0 }
                .joined(separator: " ")
            let isSecret = jsFlaggedSecret || FabushiBrowserSecurity.isSensitiveParameterName(locatorHint)
            let rawValue = (payload["value"] as? String) ?? ""
            let safeValue = isSecret ? "[REDACTED]" : rawValue
            return .fill(
                locator: locator,
                value: safeValue,
                isSecret: isSecret,
                parameterNameHint: hint
            )
        case "press_key":
            guard let key = payload["key"] as? String, !key.isEmpty else { return nil }
            return .pressKey(key: key)
        case "scroll":
            let dx = (payload["delta_x"] as? NSNumber)?.doubleValue ?? 0
            let dy = (payload["delta_y"] as? NSNumber)?.doubleValue ?? 0
            return .scroll(deltaX: dx, deltaY: dy)
        default:
            return nil
        }
    }

    // MARK: - Zero-LLM Deterministic Script Step Execution

    public static func renderTemplate(
        _ template: String,
        parameters: [String: String]
    ) throws -> String {
        let pattern = #"\{\{([a-zA-Z0-9_.-]+)\}\}"#
        let regex = try NSRegularExpression(pattern: pattern)
        let nsTemplate = template as NSString
        let matches = regex.matches(in: template, range: NSRange(location: 0, length: nsTemplate.length))
        var result = template
        for match in matches.reversed() {
            guard match.numberOfRanges >= 2 else { continue }
            let paramName = nsTemplate.substring(with: match.range(at: 1))
            guard let replacement = parameters[paramName] else {
                throw FabushiBrowserRunnerError.missingRequiredParameter(paramName)
            }
            if let range = Range(match.range, in: result) {
                result.replaceSubrange(range, with: replacement)
            }
        }
        return result
    }

    public func executeStep(
        _ step: FabushiScriptStep,
        resolvedParams: [String: String],
        extractedOutputs: inout [String: String]
    ) async throws {
        switch step {
        case .navigate(let urlTemplate):
            let urlStr = try Self.renderTemplate(urlTemplate, parameters: resolvedParams)
            try navigate(to: urlStr)

        case .waitFor(let locator, let timeoutMs):
            let deadline = Date().addingTimeInterval(Double(max(100, timeoutMs)) / 1000.0)
            while Date() < deadline {
                let res = try await evaluateLocatorJSON(locator: locator, operation: "assert_visible", value: nil)
                if (res["ok"] as? Bool) == true {
                    return
                }
                try await Task.sleep(nanoseconds: 100_000_000)
            }
            throw FabushiBrowserRunnerError.elementNotFound(locator.highestPriorityStrategy)

        case .click(let locator):
            let res = try await evaluateLocatorJSON(locator: locator, operation: "click", value: nil)
            guard (res["ok"] as? Bool) == true else {
                throw FabushiBrowserRunnerError.elementNotFound(locator.highestPriorityStrategy)
            }

        case .fill(let locator, let valueTemplate, _):
            let resolvedValue = try Self.renderTemplate(valueTemplate, parameters: resolvedParams)
            let res = try await evaluateLocatorJSON(locator: locator, operation: "fill", value: resolvedValue)
            guard (res["ok"] as? Bool) == true else {
                throw FabushiBrowserRunnerError.elementNotFound(locator.highestPriorityStrategy)
            }

        case .pressKey(let key):
            let escaped = key.replacingOccurrences(of: "'", with: "\\'")
            let js = "document.dispatchEvent(new KeyboardEvent('keydown', { key: '\(escaped)', bubbles: true })); 'ok';"
            _ = try? await evaluateJavaScriptString(js)

        case .scroll(let dx, let dy):
            let js = "window.scrollBy(\(dx), \(dy)); 'ok';"
            _ = try? await evaluateJavaScriptString(js)

        case .extractText(let locator, let outputKey):
            let res = try await evaluateLocatorJSON(locator: locator, operation: "extract_text", value: nil)
            guard (res["ok"] as? Bool) == true else {
                throw FabushiBrowserRunnerError.elementNotFound(locator.highestPriorityStrategy)
            }
            extractedOutputs[outputKey] = (res["text"] as? String) ?? ""

        case .assertVisible(let locator):
            let res = try await evaluateLocatorJSON(locator: locator, operation: "assert_visible", value: nil)
            guard (res["ok"] as? Bool) == true else {
                throw FabushiBrowserRunnerError.elementNotFound(locator.highestPriorityStrategy)
            }

        case .assertTextContains(let locator, let expectedTemplate):
            let expected = try Self.renderTemplate(expectedTemplate, parameters: resolvedParams)
            let res = try await evaluateLocatorJSON(locator: locator, operation: "assert_text_contains", value: expected)
            guard (res["ok"] as? Bool) == true else {
                throw FabushiBrowserRunnerError.scriptExecutionFailed("Text assertion failed for \(locator.highestPriorityStrategy)")
            }

        case .humanCheckpoint(let reason):
            throw FabushiBrowserRunnerError.humanCheckpointPaused(reason)
        }
    }

    private func evaluateJavaScriptString(_ js: String) async throws -> String {
        try await withCheckedThrowingContinuation { continuation in
            webView.evaluateJavaScript(js) { value, error in
                if let error {
                    continuation.resume(throwing: error)
                } else if let str = value as? String {
                    continuation.resume(returning: str)
                } else {
                    continuation.resume(returning: "")
                }
            }
        }
    }

    private func evaluateLocatorJSON(
        locator: FabushiBrowserSemanticLocator,
        operation: String,
        value: String?
    ) async throws -> [String: Any] {
        let js = FabushiBrowserScripts.semanticLocatorOperationJS(
            locator: locator,
            operation: operation,
            value: value
        )
        let str = try await evaluateJavaScriptString(js)
        guard let data = str.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data, options: []) as? [String: Any]
        else {
            throw FabushiBrowserRunnerError.scriptExecutionFailed("Invalid JS bridge result")
        }
        return obj
    }
}
