import Foundation

// MARK: - Logout & Server-Switch Cleanup Hub (`D10-10` `reset_on_logout()`)

/// Coordinates account-scoped and server-switch state resets across all active Milestone M4 ViewModels.
/// Installs idempotently onto `FabushiCoordinator`'s departing-account & server-switch purger hooks.
@MainActor
public final class FabushiAgentStudioLogoutCleanupHub {
    public static let coordinatorPurgerId = "fabushi.agentStudio.cleanup"

    public enum ResetReason: Sendable, Equatable {
        case accountLogout(accountId: String)
        case serverSwitch
        case generationAdvance(UInt64)
        case manual
    }

    private var handlers: [String: @MainActor (ResetReason) -> Void] = [:]

    public init() {}

    public var registeredHandlerCount: Int {
        handlers.count
    }

    public func registerResetHandler(
        id: String,
        handler: @escaping @MainActor (ResetReason) -> Void
    ) {
        handlers[id] = handler
    }

    public func unregisterResetHandler(id: String) {
        handlers.removeValue(forKey: id)
    }

    /// Idempotently binds this hub to `FabushiCoordinator`'s logout and server-switch purgers.
    public func install(on coordinator: FabushiCoordinator) {
        coordinator.registerAccountSecretsPurger(id: Self.coordinatorPurgerId) { [weak self] accountId in
            self?.resetForDepartingAccount(accountId)
        }
        coordinator.registerServerSwitchSecretsPurger(id: Self.coordinatorPurgerId) { [weak self] in
            self?.resetForServerSwitch()
        }
    }

    public func resetForDepartingAccount(_ accountId: String) {
        triggerReset(reason: .accountLogout(accountId: accountId))
    }

    public func resetForServerSwitch() {
        triggerReset(reason: .serverSwitch)
    }

    public func triggerReset(reason: ResetReason) {
        let snapshot = Array(handlers.values)
        for handler in snapshot {
            handler(reason)
        }
    }
}

// MARK: - 6-Stage Secret & JWT Trajectory Redactor (`D11-05` `hermes/trajectory.rs:L122-L486`)

public struct FabushiStudioRedactionResult: Sendable, Equatable {
    public let redactedText: String
    public let findingsByCategory: [String: Int]
    public let totalRedactions: Int

    public init(redactedText: String, findingsByCategory: [String: Int], totalRedactions: Int) {
        self.redactedText = redactedText
        self.findingsByCategory = findingsByCategory
        self.totalRedactions = totalRedactions
    }
}

/// Exact 6-stage pure-Swift mirror of `gpui_telegram_agent::hermes::trajectory::SecretRedactor`.
public struct FabushiStudioSecretRedactor: Sendable {
    public let customLiterals: [String]

    private struct PrefixedRule: Sendable {
        let prefixScalars: [UnicodeScalar]
        let minTotalLen: Int
        let marker: String
        let category: String

        init(_ prefix: String, _ minTotalLen: Int, _ marker: String, _ category: String) {
            self.prefixScalars = Array(prefix.unicodeScalars)
            self.minTotalLen = minTotalLen
            self.marker = marker
            self.category = category
        }
    }

    private static let prefixedRules: [PrefixedRule] = [
        PrefixedRule("sk-ant-", 16, "[REDACTED:ANTHROPIC_KEY]", "anthropic_key"),
        PrefixedRule("sk-proj-", 16, "[REDACTED:OPENAI_KEY]", "openai_key"),
        PrefixedRule("sk-", 18, "[REDACTED:API_KEY]", "api_key"),
        PrefixedRule("github_pat_", 18, "[REDACTED:GITHUB_TOKEN]", "github_token"),
        PrefixedRule("ghp_", 16, "[REDACTED:GITHUB_TOKEN]", "github_token"),
        PrefixedRule("gho_", 16, "[REDACTED:GITHUB_TOKEN]", "github_token"),
        PrefixedRule("ghs_", 16, "[REDACTED:GITHUB_TOKEN]", "github_token"),
        PrefixedRule("ghu_", 16, "[REDACTED:GITHUB_TOKEN]", "github_token"),
        PrefixedRule("xoxb-", 14, "[REDACTED:SLACK_TOKEN]", "slack_token"),
        PrefixedRule("xoxp-", 14, "[REDACTED:SLACK_TOKEN]", "slack_token"),
        PrefixedRule("xoxa-", 14, "[REDACTED:SLACK_TOKEN]", "slack_token"),
        PrefixedRule("xoxr-", 14, "[REDACTED:SLACK_TOKEN]", "slack_token"),
        PrefixedRule("AIza", 24, "[REDACTED:GOOGLE_API_KEY]", "google_api_key"),
        PrefixedRule("AKIA", 18, "[REDACTED:AWS_ACCESS_KEY]", "aws_access_key"),
        PrefixedRule("ASIA", 18, "[REDACTED:AWS_ACCESS_KEY]", "aws_access_key"),
    ]

    private static let kvSecretKeys: [String] = [
        "aws_secret_access_key",
        "openai_api_key",
        "anthropic_api_key",
        "openrouter_api_key",
        "refresh_token",
        "oauth_token",
        "access_token",
        "auth_token",
        "session_id",
        "client_secret",
        "secret_key",
        "set-cookie",
        "x-api-key",
        "password",
        "passcode",
        "passwd",
        "api_key",
        "apikey",
        "id_token",
        "cookie",
        "secret",
        "token",
        "totp",
        "otp",
        "ct0",
        "pwd",
    ]

    private static let shortBoundaryKvKeys: Set<String> = ["otp", "ct0", "pwd", "totp"]
    private static let benignKvLiterals: Set<String> = ["true", "false", "null", "none"]

    public init(customLiterals: [String] = []) {
        self.customLiterals = customLiterals.filter { !$0.isEmpty }
    }

    public func redactText(_ input: String) -> FabushiStudioRedactionResult {
        var counts: [String: Int] = [:]
        var total = 0

        func record(_ category: String) {
            counts[category, default: 0] += 1
            total += 1
        }

        var text = input

        // Stage 1: Custom secret literals
        for lit in customLiterals where !lit.isEmpty {
            if text.contains(lit) {
                var remaining = text[...]
                var rebuilt = ""
                rebuilt.reserveCapacity(text.count)
                while let range = remaining.range(of: lit) {
                    rebuilt.append(contentsOf: remaining[..<range.lowerBound])
                    rebuilt.append("[REDACTED:CUSTOM_SECRET]")
                    record("custom_secret")
                    remaining = remaining[range.upperBound...]
                }
                rebuilt.append(contentsOf: remaining)
                text = rebuilt
            }
        }

        // Stage 2: PEM private key blocks
        text = redactPemBlocks(text, record: record)

        // Stage 3: Bearer & Basic authorization headers
        text = redactAuthScheme(
            text,
            prefix: "Bearer ",
            replacement: "[REDACTED:BEARER_TOKEN]",
            category: "bearer_token",
            record: record
        )
        text = redactAuthScheme(
            text,
            prefix: "bearer ",
            replacement: "[REDACTED:BEARER_TOKEN]",
            category: "bearer_token",
            record: record
        )
        text = redactAuthScheme(
            text,
            prefix: "Basic ",
            replacement: "[REDACTED:BASIC_AUTH]",
            category: "basic_auth",
            record: record
        )

        // Stage 4: 15 prefixed API keys / tokens
        text = redactPrefixedTokens(text, record: record)

        // Stage 5: Raw JWT tokens (`eyJ<p1>.<p2>.<p3>`)
        text = redactJwtTokens(text, record: record)

        // Stage 6: 26 key-value secret assignments
        text = redactKeyValueSecrets(text, record: record)

        return FabushiStudioRedactionResult(
            redactedText: text,
            findingsByCategory: counts,
            totalRedactions: total
        )
    }

    // MARK: - Pass Helpers

    private static func isAsciiAlnum(_ s: UnicodeScalar) -> Bool {
        let v = s.value
        return (v >= 0x30 && v <= 0x39)
            || (v >= 0x41 && v <= 0x5A)
            || (v >= 0x61 && v <= 0x7A)
    }

    private static func isTokenScalar(_ s: UnicodeScalar) -> Bool {
        if isAsciiAlnum(s) { return true }
        switch s.value {
        case 0x5F, 0x2D, 0x2E, 0x3D, 0x2B, 0x2F: // _ - . = + /
            return true
        default:
            return false
        }
    }

    private static func isJwtScalar(_ s: UnicodeScalar) -> Bool {
        if isAsciiAlnum(s) { return true }
        switch s.value {
        case 0x5F, 0x2D, 0x2E: // _ - .
            return true
        default:
            return false
        }
    }

    private static func isAsciiWhitespace(_ s: UnicodeScalar) -> Bool {
        s == " " || s == "\t" || s == "\r" || s == "\n" || CharacterSet.whitespacesAndNewlines.contains(s)
    }

    private static func asciiLower(_ s: UnicodeScalar) -> UnicodeScalar {
        let v = s.value
        if v >= 0x41 && v <= 0x5A, let lowered = UnicodeScalar(v + 32) {
            return lowered
        }
        return s
    }

    private static func matchesPrefix(
        _ scalars: [UnicodeScalar],
        at index: Int,
        prefix: [UnicodeScalar]
    ) -> Bool {
        guard index + prefix.count <= scalars.count else { return false }
        for offset in 0..<prefix.count {
            if scalars[index + offset] != prefix[offset] {
                return false
            }
        }
        return true
    }

    private func redactPemBlocks(_ input: String, record: (String) -> Void) -> String {
        var out = ""
        out.reserveCapacity(input.count)
        var rest = input[...]
        let beginMarker = "-----BEGIN "
        let endMarker = "-----END "
        let dashes = "-----"

        while let startRange = rest.range(of: beginMarker) {
            out.append(contentsOf: rest[..<startRange.lowerBound])
            let afterStart = rest[startRange.lowerBound...]
            if let endMarkerRange = afterStart.range(of: endMarker) {
                let afterEndMarker = afterStart[endMarkerRange.upperBound...]
                if let endDashesRange = afterEndMarker.range(of: dashes) {
                    out.append("[REDACTED:PRIVATE_KEY]")
                    record("private_key")
                    rest = afterStart[endDashesRange.upperBound...]
                    continue
                }
            }
            out.append(beginMarker)
            rest = rest[startRange.upperBound...]
        }
        out.append(contentsOf: rest)
        return out
    }

    private func redactAuthScheme(
        _ input: String,
        prefix: String,
        replacement: String,
        category: String,
        record: (String) -> Void
    ) -> String {
        var out = ""
        out.reserveCapacity(input.count)
        var rest = input[...]

        while let range = rest.range(of: prefix) {
            out.append(contentsOf: rest[..<range.lowerBound])
            let after = rest[range.upperBound...]

            var tokEnd = after.startIndex
            var tokBytes = 0
            for scalar in after.unicodeScalars {
                guard Self.isTokenScalar(scalar) else { break }
                tokBytes += scalar.utf8.count
                tokEnd = after.unicodeScalars.index(after: tokEnd)
            }

            let token = after[..<tokEnd]
            if tokBytes >= 8 && !token.hasPrefix("[REDACTED") {
                out.append(prefix)
                out.append(replacement)
                record(category)
                rest = after[tokEnd...]
            } else {
                out.append(prefix)
                rest = after
            }
        }
        out.append(contentsOf: rest)
        return out
    }

    private func redactPrefixedTokens(_ input: String, record: (String) -> Void) -> String {
        let scalars = Array(input.unicodeScalars)
        var out = String.UnicodeScalarView()
        out.reserveCapacity(scalars.count)

        var i = 0
        while i < scalars.count {
            let prevOk: Bool
            if i == 0 {
                prevOk = true
            } else {
                let p = scalars[i - 1]
                prevOk = !Self.isAsciiAlnum(p) && p != "_"
            }

            var matched = false
            if prevOk {
                for rule in Self.prefixedRules {
                    if Self.matchesPrefix(scalars, at: i, prefix: rule.prefixScalars) {
                        var j = i
                        var tokBytes = 0
                        while j < scalars.count,
                              Self.isTokenScalar(scalars[j]),
                              scalars[j] != "="
                        {
                            tokBytes += scalars[j].utf8.count
                            j += 1
                        }
                        if tokBytes >= rule.minTotalLen {
                            out.append(contentsOf: rule.marker.unicodeScalars)
                            record(rule.category)
                            i = j
                            matched = true
                            break
                        }
                    }
                }
            }

            if !matched {
                out.append(scalars[i])
                i += 1
            }
        }
        return String(out)
    }

    private func redactJwtTokens(_ input: String, record: (String) -> Void) -> String {
        let scalars = Array(input.unicodeScalars)
        let eyjPrefix = Array("eyJ".unicodeScalars)
        var out = String.UnicodeScalarView()
        out.reserveCapacity(scalars.count)

        var i = 0
        while i < scalars.count {
            let prevOk: Bool
            if i == 0 {
                prevOk = true
            } else {
                let p = scalars[i - 1]
                prevOk = !Self.isAsciiAlnum(p) && p != "_"
            }

            var matched = false
            if prevOk && Self.matchesPrefix(scalars, at: i, prefix: eyjPrefix) {
                var j = i
                while j < scalars.count && Self.isJwtScalar(scalars[j]) {
                    j += 1
                }
                let token = String(String.UnicodeScalarView(scalars[i..<j]))
                let parts = token.split(separator: ".", omittingEmptySubsequences: false)
                if token.utf8.count >= 24,
                   parts.count == 3,
                   parts[0].utf8.count >= 4,
                   parts[1].utf8.count >= 4,
                   parts[2].utf8.count >= 4
                {
                    out.append(contentsOf: "[REDACTED:JWT]".unicodeScalars)
                    record("jwt")
                    i = j
                    matched = true
                }
            }

            if !matched {
                out.append(scalars[i])
                i += 1
            }
        }
        return String(out)
    }

    private func redactKeyValueSecrets(_ input: String, record: (String) -> Void) -> String {
        let redactedMarker = Array("[REDACTED:SECRET]".unicodeScalars)
        let redactedPrefix = Array("[REDACTED".unicodeScalars)
        let lines = input.unicodeScalars.split(separator: "\n", omittingEmptySubsequences: false)
        var outLines: [String] = []
        outLines.reserveCapacity(lines.count)

        for rawLine in lines {
            var scalars = Array(rawLine)

            for key in Self.kvSecretKeys {
                let keyScalars = Array(key.unicodeScalars)
                let isShortKey = Self.shortBoundaryKvKeys.contains(key)
                var searchFrom = 0

                while searchFrom < scalars.count {
                    let lowerScalars = scalars.map(Self.asciiLower)
                    guard let pos = findSubsequence(keyScalars, in: lowerScalars, from: searchFrom) else {
                        break
                    }

                    let prevOk: Bool
                    if pos == 0 {
                        prevOk = true
                    } else {
                        let prev = lowerScalars[pos - 1]
                        prevOk = isShortKey
                            ? (!Self.isAsciiAlnum(prev) && prev != "_")
                            : !Self.isAsciiAlnum(prev)
                    }
                    guard prevOk else {
                        searchFrom = pos + keyScalars.count
                        continue
                    }

                    var cursor = pos + keyScalars.count
                    if cursor < scalars.count && (scalars[cursor] == "\"" || scalars[cursor] == "'") {
                        cursor += 1
                    }
                    while cursor < scalars.count && Self.isAsciiWhitespace(scalars[cursor]) {
                        cursor += 1
                    }
                    guard cursor < scalars.count && (scalars[cursor] == "=" || scalars[cursor] == ":") else {
                        searchFrom = pos + keyScalars.count
                        continue
                    }
                    cursor += 1 // consume '=' or ':'
                    while cursor < scalars.count && Self.isAsciiWhitespace(scalars[cursor]) {
                        cursor += 1
                    }

                    let valueQuoteStart = cursor
                    var valStart = valueQuoteStart
                    if valStart < scalars.count && (scalars[valStart] == "\"" || scalars[valStart] == "'") {
                        valStart += 1
                    }

                    var valEnd = valStart
                    while valEnd < scalars.count {
                        let c = scalars[valEnd]
                        if Self.isAsciiWhitespace(c)
                            || c == "\"" || c == "'" || c == "," || c == ";"
                            || c == "}" || c == "&" || c == "#"
                        {
                            break
                        }
                        valEnd += 1
                    }

                    let valSlice = scalars[valStart..<valEnd]
                    let valBytes = valSlice.reduce(0) { $0 + $1.utf8.count }
                    let candidateLower = String(String.UnicodeScalarView(valSlice.map(Self.asciiLower)))
                    let alreadyRedacted = Self.matchesPrefix(scalars, at: valStart, prefix: redactedPrefix)

                    if valBytes >= 4 && !Self.benignKvLiterals.contains(candidateLower) && !alreadyRedacted {
                        scalars.replaceSubrange(valStart..<valEnd, with: redactedMarker)
                        record("key_value_secret")
                        searchFrom = valStart + redactedMarker.count
                    } else {
                        searchFrom = pos + keyScalars.count
                    }
                }
            }

            outLines.append(String(String.UnicodeScalarView(scalars)))
        }

        return outLines.joined(separator: "\n")
    }

    private func findSubsequence(
        _ needle: [UnicodeScalar],
        in haystack: [UnicodeScalar],
        from startIndex: Int
    ) -> Int? {
        guard !needle.isEmpty, startIndex + needle.count <= haystack.count else {
            return nil
        }
        let limit = haystack.count - needle.count
        var i = startIndex
        while i <= limit {
            var matched = true
            for offset in 0..<needle.count {
                if haystack[i + offset] != needle[offset] {
                    matched = false
                    break
                }
            }
            if matched { return i }
            i += 1
        }
        return nil
    }
}

// MARK: - `AutomationScriptApp` Publish Preflight Validator (`D11-04` `validate_script_app_safe_for_publish`)

public enum FabushiStudioScriptAppPreflightIssueKind: String, Sendable, Equatable {
    case scriptValidation = "script_validation"
    case secretLeakDetected = "secret_leak_detected"
}

public struct FabushiStudioScriptAppPreflightIssue: Identifiable, Sendable, Equatable {
    public let id: String
    public let kind: FabushiStudioScriptAppPreflightIssueKind
    public let path: String
    public let message: String

    public init(kind: FabushiStudioScriptAppPreflightIssueKind, path: String, message: String) {
        self.id = "\(kind.rawValue):\(path):\(message)"
        self.kind = kind
        self.path = path
        self.message = message
    }
}

public struct FabushiStudioScriptAppPreflightResult: Sendable, Equatable {
    public let isValid: Bool
    public let issues: [FabushiStudioScriptAppPreflightIssue]

    public init(issues: [FabushiStudioScriptAppPreflightIssue]) {
        self.isValid = issues.isEmpty
        self.issues = issues
    }
}

/// Exact client-side mirror of Rust `bot_browser::script_app::validate_script_app_safe_for_publish`.
public enum FabushiStudioScriptAppPreflightValidator: Sendable {
    public static let maxJsonDepth = 16
    public static let maxDecodePasses = 4
    public static let maxDecodeVariants = 64

    public static let sensitiveKvKeys: Set<String> = [
        "auth_token", "auth-token", "ct0", "password", "passwd", "passcode", "pwd",
        "api_key", "api-key", "apikey", "x-api-key", "x_api_key",
        "access_token", "refresh_token", "id_token", "session_id",
        "set-cookie", "cookie", "authorization", "token", "secret",
    ]

    private static let sensitiveParamSubstrings: [String] = [
        "ct0", "password", "passwd", "passcode", "pwd",
        "apikey", "authorization", "token", "secret", "cookie",
        "bearer", "basic", "pin", "otp", "totp",
    ]

    private static let sensitiveParamSegmentPairs: [(String, String)] = [
        ("ct", "0"), ("auth", "token"), ("api", "key"),
        ("access", "token"), ("refresh", "token"), ("id", "token"),
        ("session", "id"), ("set", "cookie"), ("private", "key"),
        ("pass", "word"), ("pass", "wd"), ("pass", "code"),
    ]

    private static let blockedMetadataHosts: Set<String> = [
        "169.254.169.254",
        "metadata.google.internal",
    ]

    public static func isSensitiveParamName(_ rawName: String) -> Bool {
        let lower = rawName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !lower.isEmpty else { return false }

        let segments = lower
            .split { !($0.isLetter || $0.isNumber) }
            .map(String.init)
            .filter { !$0.isEmpty }

        for sub in sensitiveParamSubstrings {
            if sub == "pin" || sub == "otp" || sub == "pwd" || sub == "ct0" {
                if segments.contains(sub) || lower == sub {
                    return true
                }
            } else if lower.contains(sub) {
                return true
            }
        }

        if segments.count >= 2 {
            for idx in 0..<(segments.count - 1) {
                let pair = (segments[idx], segments[idx + 1])
                if sensitiveParamSegmentPairs.contains(where: { $0.0 == pair.0 && $0.1 == pair.1 }) {
                    return true
                }
            }
        }
        return false
    }

    public static func validate(_ app: FabushiStudioAutomationScriptApp) -> FabushiStudioScriptAppPreflightResult {
        var issues: [FabushiStudioScriptAppPreflightIssue] = []

        // 1. Structural integrity
        if app.schemaVersion < 1 {
            issues.append(.init(kind: .scriptValidation, path: "schema_version", message: "schema_version must be >= 1"))
        }
        if app.appId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            issues.append(.init(kind: .scriptValidation, path: "app_id", message: "app_id cannot be empty"))
        }
        if app.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            issues.append(.init(kind: .scriptValidation, path: "name", message: "name cannot be empty"))
        }
        if app.version.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            issues.append(.init(kind: .scriptValidation, path: "version", message: "version cannot be empty"))
        }
        if app.steps.isEmpty {
            issues.append(.init(kind: .scriptValidation, path: "steps", message: "steps cannot be empty"))
        }
        let slug = app.slug
        let slugValid = !slug.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && slug.unicodeScalars.allSatisfy { s in
                let v = s.value
                return (v >= 0x61 && v <= 0x7A) // a-z
                    || (v >= 0x30 && v <= 0x39) // 0-9
                    || v == 0x2D                // -
                    || v == 0x5F                // _
            }
        if !slugValid {
            issues.append(.init(
                kind: .scriptValidation,
                path: "slug",
                message: "invalid slug '\(slug)': must be non-empty [a-z0-9_-]"
            ))
        }

        // 2. Parameter hygiene
        var paramMap: [String: FabushiStudioScriptParameter] = [:]
        for p in app.parameters {
            let pname = p.name.trimmingCharacters(in: .whitespacesAndNewlines)
            if pname.isEmpty {
                issues.append(.init(kind: .scriptValidation, path: "parameters", message: "parameter name cannot be empty"))
                continue
            }
            if paramMap[pname] != nil {
                issues.append(.init(kind: .scriptValidation, path: "parameters.\(pname)", message: "duplicate parameter name '\(pname)'"))
            } else {
                paramMap[pname] = p
            }

            let hasDefault = !(p.defaultValue ?? "").isEmpty
            if p.isSecret && hasDefault {
                issues.append(.init(
                    kind: .secretLeakDetected,
                    path: "parameters.\(pname)",
                    message: "secret parameter '\(pname)' must not carry a default_value"
                ))
            }
            if isSensitiveParamName(pname) && (!p.isSecret || hasDefault) {
                issues.append(.init(
                    kind: .secretLeakDetected,
                    path: "parameters.\(pname)",
                    message: "sensitive parameter '\(pname)' must be marked is_secret=true with no default_value"
                ))
            }
        }

        // 3. Recursive JSON secret-leak scan
        if let encoded = try? JSONEncoder().encode(app),
           let jsonValue = try? JSONDecoder().decode(FabushiJSONValue.self, from: encoded)
        {
            scanJsonValue(jsonValue, path: "app", depth: 0, paramMap: paramMap, issues: &issues)
        } else {
            issues.append(.init(kind: .scriptValidation, path: "app", message: "failed to serialize app for validation"))
        }

        // 4. Per-step validation (all 10 variants)
        for (idx, step) in app.steps.enumerated() {
            let stepPath = "steps[\(idx)]"
            switch step {
            case .navigate(let urlTemplate):
                let dummyUrl = replacePlaceholders(in: urlTemplate, with: "example")
                if let urlError = validateNavigateURL(dummyUrl) {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: urlError))
                }

            case .waitFor(let locator, let timeoutMs):
                if locator.isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "WaitFor locator cannot be empty"))
                }
                if timeoutMs == 0 {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "WaitFor timeout_ms must be > 0"))
                }

            case .click(let locator), .assertVisible(let locator):
                if locator.isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "\(step.kindName) locator cannot be empty"))
                }

            case .fill(let locator, let valueTemplate, let secretVaultKey):
                if locator.isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "Fill locator cannot be empty"))
                }
                let refsSecret = templateReferencesSecretParam(valueTemplate, paramMap: paramMap)
                if locator.isSensitiveHint || refsSecret {
                    let hasVaultKey = !(secretVaultKey ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    let pureSecret = isProtectedSecretPlaceholder(valueTemplate, paramMap: paramMap)
                    let safeWithVault = hasVaultKey
                        && (valueTemplate.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || pureSecret)
                    if !safeWithVault && !pureSecret {
                        issues.append(.init(
                            kind: .secretLeakDetected,
                            path: stepPath,
                            message: "sensitive Fill must use secret_vault_key or an is_secret=true placeholder with no default"
                        ))
                    }
                }

            case .pressKey(let key):
                if key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "PressKey cannot be empty"))
                }

            case .scroll(let dx, let dy):
                if !dx.isFinite || !dy.isFinite {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "Scroll deltas must be finite numbers"))
                }

            case .extractText(let locator, let outputKey):
                if locator.isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "ExtractText locator cannot be empty"))
                }
                if outputKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "ExtractText output_key cannot be empty"))
                }

            case .assertTextContains(let locator, _):
                if locator.isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "AssertTextContains locator cannot be empty"))
                }

            case .humanCheckpoint(let reason):
                if reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    issues.append(.init(kind: .scriptValidation, path: stepPath, message: "HumanCheckpoint reason cannot be empty"))
                }
            }
        }

        return FabushiStudioScriptAppPreflightResult(issues: issues)
    }

    // MARK: - Private Scan Helpers

    private static func scanJsonValue(
        _ value: FabushiJSONValue,
        path: String,
        depth: Int,
        paramMap: [String: FabushiStudioScriptParameter],
        issues: inout [FabushiStudioScriptAppPreflightIssue]
    ) {
        if depth > maxJsonDepth {
            issues.append(.init(kind: .secretLeakDetected, path: path, message: "JSON nesting depth exceeded \(maxJsonDepth)"))
            return
        }
        switch value {
        case .object(let dict):
            for (k, v) in dict {
                let childPath = "\(path).\(k)"
                let lowerKey = k.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                if sensitiveKvKeys.contains(lowerKey),
                   case .string(let rawVal) = v
                {
                    let trimmed = rawVal.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !trimmed.isEmpty && !isAllowedSecretFieldValue(trimmed, key: lowerKey, paramMap: paramMap) {
                        issues.append(.init(
                            kind: .secretLeakDetected,
                            path: childPath,
                            message: "raw secret literal under sensitive key '\(k)'"
                        ))
                    }
                }
                scanJsonValue(v, path: childPath, depth: depth + 1, paramMap: paramMap, issues: &issues)
            }
        case .array(let items):
            for (idx, item) in items.enumerated() {
                scanJsonValue(item, path: "\(path)[\(idx)]", depth: depth + 1, paramMap: paramMap, issues: &issues)
            }
        case .string(let text):
            scanStringForLeaks(text, path: path, paramMap: paramMap, issues: &issues)
        case .null, .bool, .int, .double:
            break
        }
    }

    private static func scanStringForLeaks(
        _ raw: String,
        path: String,
        paramMap: [String: FabushiStudioScriptParameter],
        issues: inout [FabushiStudioScriptAppPreflightIssue]
    ) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if isProtectedSecretPlaceholder(trimmed, paramMap: paramMap) {
            return
        }

        let redactor = FabushiStudioSecretRedactor()
        let variants = decodeVariants(trimmed)
        for variant in variants {
            let result = redactor.redactText(variant)
            if result.totalRedactions > 0 {
                let categories = result.findingsByCategory.keys.sorted().joined(separator: ", ")
                issues.append(.init(
                    kind: .secretLeakDetected,
                    path: path,
                    message: "potential secret leak detected (\(categories))"
                ))
                return
            }
        }
    }

    private static func decodeVariants(_ input: String) -> [String] {
        var seen: Set<String> = [input]
        var queue: [String] = [input]
        var cursor = 0

        for _ in 0..<maxDecodePasses {
            let levelEnd = queue.count
            if cursor >= levelEnd || queue.count >= maxDecodeVariants { break }
            while cursor < levelEnd && queue.count < maxDecodeVariants {
                let current = queue[cursor]
                cursor += 1
                var nextCandidates: [String] = []
                if let pct = current.removingPercentEncoding, pct != current {
                    nextCandidates.append(pct)
                }
                if current.contains("+") {
                    let plusSpaced = current.replacingOccurrences(of: "+", with: " ")
                    if let pct = plusSpaced.removingPercentEncoding, pct != current {
                        nextCandidates.append(pct)
                    }
                }
                if current.contains("\\") {
                    let unescaped = unescapeJsonSequences(current)
                    if unescaped != current {
                        nextCandidates.append(unescaped)
                    }
                }
                for candidate in nextCandidates where seen.insert(candidate).inserted {
                    if queue.count < maxDecodeVariants {
                        queue.append(candidate)
                    }
                }
            }
        }
        return queue
    }

    private static func unescapeJsonSequences(_ input: String) -> String {
        let wrapped = "\"\(input.replacingOccurrences(of: "\"", with: "\\\""))\""
        if let data = wrapped.data(using: .utf8),
           let decoded = try? JSONDecoder().decode(String.self, from: data)
        {
            return decoded
        }
        return input
            .replacingOccurrences(of: "\\\"", with: "\"")
            .replacingOccurrences(of: "\\\\", with: "\\")
    }

    private static func extractPurePlaceholder(_ raw: String) -> String? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{{"), trimmed.hasSuffix("}}"), trimmed.count > 4 else {
            return nil
        }
        let inner = String(trimmed.dropFirst(2).dropLast(2))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !inner.isEmpty, !inner.contains("{"), !inner.contains("}") else {
            return nil
        }
        return inner
    }

    private static func isProtectedSecretPlaceholder(
        _ raw: String,
        paramMap: [String: FabushiStudioScriptParameter]
    ) -> Bool {
        guard let name = extractPurePlaceholder(raw),
              let param = paramMap[name]
        else {
            return false
        }
        return param.isSecret && (param.defaultValue?.isEmpty ?? true)
    }

    private static func isAllowedSecretFieldValue(
        _ trimmed: String,
        key: String,
        paramMap: [String: FabushiStudioScriptParameter]
    ) -> Bool {
        if isProtectedSecretPlaceholder(trimmed, paramMap: paramMap) {
            return true
        }
        if key == "authorization" {
            for scheme in ["Bearer ", "bearer ", "Basic ", "basic "] {
                if trimmed.hasPrefix(scheme) {
                    let tail = String(trimmed.dropFirst(scheme.count))
                    if isProtectedSecretPlaceholder(tail, paramMap: paramMap) {
                        return true
                    }
                }
            }
        }
        return false
    }

    private static func templateReferencesSecretParam(
        _ template: String,
        paramMap: [String: FabushiStudioScriptParameter]
    ) -> Bool {
        var rest = template[...]
        while let open = rest.range(of: "{{") {
            let afterOpen = rest[open.upperBound...]
            guard let close = afterOpen.range(of: "}}") else { break }
            let name = String(afterOpen[..<close.lowerBound])
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if let param = paramMap[name], param.isSecret {
                return true
            }
            rest = afterOpen[close.upperBound...]
        }
        return false
    }

    private static func replacePlaceholders(in template: String, with replacement: String) -> String {
        var out = ""
        var rest = template[...]
        while let open = rest.range(of: "{{") {
            out.append(contentsOf: rest[..<open.lowerBound])
            let afterOpen = rest[open.upperBound...]
            guard let close = afterOpen.range(of: "}}") else {
                out.append(contentsOf: rest[open.lowerBound...])
                return out
            }
            out.append(replacement)
            rest = afterOpen[close.upperBound...]
        }
        out.append(contentsOf: rest)
        return out
    }

    private static func validateNavigateURL(_ rawUrl: String) -> String? {
        let trimmed = rawUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https"
        else {
            return "invalid navigate url_template: must use http:// or https://"
        }
        if components.user != nil || components.password != nil {
            return "invalid navigate url_template: embedded userinfo is forbidden"
        }
        guard let host = components.host?.lowercased(), !host.isEmpty else {
            return "invalid navigate url_template: missing host"
        }
        if blockedMetadataHosts.contains(host) {
            return "blocked metadata URL host '\(host)'"
        }
        return nil
    }
}

// MARK: - 3-Tier AI Provider Error Classifier (`D10-10` `shell_bot_browser.rs:L4358-L4473`)

public enum FabushiMagpieErrorTier: String, Sendable, Equatable {
    /// Tier 1: Missing provider or subscription -> shows "Open AI Provider Settings" CTA (`hermes:open_ai_providers`).
    case unconfiguredProvider = "unconfigured_provider"
    /// Tier 2: Upstream HTTP 4xx/5xx/429, rate-limit, auth, or transport failure -> NO misleading unconfigured CTA.
    case providerRequestFailed = "provider_request_failed"
    /// Tier 3: Local turn execution failure -> NO misleading unconfigured CTA.
    case localTurnFailed = "local_turn_failed"
}

public struct FabushiMagpieClassifiedError: Sendable, Equatable {
    public let tier: FabushiMagpieErrorTier
    public let detailEn: String
    public let detailZh: String
    public let showSettingsCTA: Bool
    public let settingsActionCallback: String?

    public func heading(locale: FabushiLocale) -> String {
        switch tier {
        case .unconfiguredProvider:
            return FabushiAgentStudioStrings.text("m4.error.tier1.heading", locale: locale)
        case .providerRequestFailed:
            return FabushiAgentStudioStrings.text("m4.error.tier2.heading", locale: locale)
        case .localTurnFailed:
            return FabushiAgentStudioStrings.text("m4.error.tier3.heading", locale: locale)
        }
    }

    public func detail(locale: FabushiLocale) -> String {
        locale == .zhHans ? detailZh : detailEn
    }

    public func toastMessage(locale: FabushiLocale) -> String {
        switch tier {
        case .unconfiguredProvider:
            return FabushiAgentStudioStrings.text("m4.error.tier1.toast", locale: locale)
        case .providerRequestFailed:
            return FabushiAgentStudioStrings.text("m4.error.tier2.toast", locale: locale)
        case .localTurnFailed:
            return FabushiAgentStudioStrings.text("m4.error.tier3.toast", locale: locale)
        }
    }
}

public enum FabushiMagpieErrorClassifier: Sendable {
    public static let maxDetailChars = 240
    public static let openAiProvidersCallback = "hermes:open_ai_providers"

    private static let providerRequestMarkers: [String] = [
        "provider rate limited",
        "provider upstream http",
        "provider transport error",
        "provider authentication error",
        "provider protocol error",
        "all providers exhausted",
        "http 400",
        "http 401",
        "http 403",
        "http 429",
        "request rejected",
        "authorization header",
        "x-api-key",
        "unauthorized",
        "forbidden",
        "invalid_request_error",
        "authentication_error",
        "status 400",
        "status 401",
        "status 403",
        "status 429",
    ]

    public static func stripAnsiEscapeCodes(_ input: String) -> String {
        guard input.contains("\u{001B}") else { return input }
        var out = ""
        out.reserveCapacity(input.count)
        let scalars = Array(input.unicodeScalars)
        var i = 0
        while i < scalars.count {
            if scalars[i].value == 0x1B, i + 1 < scalars.count, scalars[i + 1] == "[" {
                i += 2
                while i < scalars.count {
                    let v = scalars[i].value
                    i += 1
                    if (v >= 0x41 && v <= 0x5A) || (v >= 0x61 && v <= 0x7A) {
                        break
                    }
                }
            } else {
                out.unicodeScalars.append(scalars[i])
                i += 1
            }
        }
        return out
    }

    public static func classify(
        rawMessage: String,
        needsProviderCta: Bool = false
    ) -> FabushiMagpieClassifiedError {
        let cleaned = stripAnsiEscapeCodes(rawMessage)
        let lower = cleaned.lowercased()

        let isProviderRequestMarker = providerRequestMarkers.contains { lower.contains($0) }

        let isUnconfiguredProvider = needsProviderCta
            || lower.contains("needs_provider_selection")
            || lower.contains("no ai subscription or provider configured")
            || lower.contains("no configured provider")
            || lower.contains("provider configuration error")
            || (lower.contains("missing api key") && !isProviderRequestMarker)
            || cleaned.contains("未检测到已配置的 AI 订阅或模型供应商")
            || cleaned.contains("未配置可用的 AI 供应商")

        let tier: FabushiMagpieErrorTier
        if isUnconfiguredProvider {
            tier = .unconfiguredProvider
        } else if lower.contains("provider_error")
            || lower.contains("providerrouter")
            || isProviderRequestMarker
        {
            tier = .providerRequestFailed
        } else {
            tier = .localTurnFailed
        }

        let sanitized = sanitizeDetail(cleaned)
        let detailEn = sanitized.isEmpty ? "Unknown local execution error." : sanitized
        let detailZh = sanitized.isEmpty ? "未知执行错误，请稍后重试。" : sanitized

        return FabushiMagpieClassifiedError(
            tier: tier,
            detailEn: detailEn,
            detailZh: detailZh,
            showSettingsCTA: tier == .unconfiguredProvider,
            settingsActionCallback: tier == .unconfiguredProvider ? openAiProvidersCallback : nil
        )
    }

    private static func sanitizeDetail(_ cleaned: String) -> String {
        var text = cleaned.trimmingCharacters(in: .whitespacesAndNewlines)

        // Strip leading "Internal error:" (case-insensitive)
        if text.lowercased().hasPrefix("internal error:") {
            text = String(text.dropFirst("internal error:".count))
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        // Strip leading "PROVIDER_ERROR:" (case-insensitive)
        if text.lowercased().hasPrefix("provider_error:") {
            text = String(text.dropFirst("provider_error:".count))
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }

        // Collapse runs of whitespace into a single ASCII space
        let collapsed = text
            .split(whereSeparator: { $0.isWhitespace })
            .joined(separator: " ")

        guard collapsed.count > maxDetailChars else {
            return collapsed
        }
        let endIdx = collapsed.index(collapsed.startIndex, offsetBy: maxDetailChars - 1)
        return String(collapsed[..<endIdx]) + "…"
    }
}

// MARK: - `TaskWakeupTriggered` Guard (`D10-10` `shell_bot_browser.rs:L4734-L4786`)

public struct FabushiStudioWakeupContext: Sendable, Equatable {
    public let isAuthenticated: Bool
    public let currentUserId: UInt64?
    public let requestedChatId: Int64?
    public let activeBotChatId: Int64?
    public let knownChatIds: Set<Int64>
    public let botChatIds: Set<Int64>

    public init(
        isAuthenticated: Bool,
        currentUserId: UInt64?,
        requestedChatId: Int64?,
        activeBotChatId: Int64? = nil,
        knownChatIds: Set<Int64>,
        botChatIds: Set<Int64>
    ) {
        self.isAuthenticated = isAuthenticated
        self.currentUserId = currentUserId
        self.requestedChatId = requestedChatId
        self.activeBotChatId = activeBotChatId
        self.knownChatIds = knownChatIds
        self.botChatIds = botChatIds
    }
}

public enum FabushiStudioWakeupBlockReason: String, Sendable, Equatable {
    case unauthenticated
    case invalidChatId = "invalid_chat_id"
    case chatNotFound = "chat_not_found"
    case targetNotBotChat = "target_not_bot_chat"
}

public enum FabushiStudioWakeupDecision: Sendable, Equatable {
    case allow(targetChatId: Int64, resumePrompt: String)
    case block(reason: FabushiStudioWakeupBlockReason)
}

public enum FabushiStudioWakeupGuard: Sendable {
    public static let canonicalResumePrompt = "安全凭据已完成本地填充，请继续原任务"

    public static func evaluate(_ context: FabushiStudioWakeupContext) -> FabushiStudioWakeupDecision {
        guard context.isAuthenticated, context.currentUserId != nil else {
            return .block(reason: .unauthenticated)
        }

        let resolvedChatId: Int64?
        if let requested = context.requestedChatId, requested > 0 {
            resolvedChatId = requested
        } else if let active = context.activeBotChatId, active > 0 {
            resolvedChatId = active
        } else {
            resolvedChatId = nil
        }

        guard let targetChatId = resolvedChatId, targetChatId > 0 else {
            return .block(reason: .invalidChatId)
        }
        guard context.knownChatIds.contains(targetChatId) else {
            return .block(reason: .chatNotFound)
        }
        guard context.botChatIds.contains(targetChatId) else {
            return .block(reason: .targetNotBotChat)
        }
        return .allow(targetChatId: targetChatId, resumePrompt: canonicalResumePrompt)
    }
}
