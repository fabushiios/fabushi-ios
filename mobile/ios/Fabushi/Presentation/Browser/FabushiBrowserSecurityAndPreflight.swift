import CryptoKit
import Foundation

// MARK: - Browser Security, Preflight Secret Scanner & Cookie Codec (`D11-06..D11-09`)

public enum FabushiBrowserPreflightError: LocalizedError, Sendable, Equatable {
    case invalidScript(String)
    case secretLeakDetected([FabushiBrowserSecretLeakFinding])
    case invalidCookiePayload(String)
    case cookiePayloadTooLarge(actualBytes: Int, maxBytes: Int)
    case vaultBundleTooLarge(actualBytes: Int, maxBytes: Int)

    public var errorDescription: String? {
        switch self {
        case .invalidScript(let message):
            return message
        case .secretLeakDetected(let findings):
            let details = findings.map { "\($0.fieldPath) [\($0.rule)]: \($0.summary)" }.joined(separator: "; ")
            return "Secret leak detected: \(details)"
        case .invalidCookiePayload(let message):
            return message
        case .cookiePayloadTooLarge(let actual, let max):
            return "Cookie payload \(actual) bytes exceeds maximum \(max) bytes."
        case .vaultBundleTooLarge(let actual, let max):
            return "Vault profile bundle \(actual) bytes exceeds 512 KiB (\(max) bytes) cap."
        }
    }
}

public enum FabushiBrowserSecurity: Sendable {
    public static let maxCookiesJSONBytes: Int = 16 * 1024 * 1024 // 16 MiB
    public static let maxVaultBundleBytes: Int = 512 * 1024 // 512 KiB (`524_288` bytes)
    public static let vaultBrowserCookieCategory: String = "browser_cookie"

    // MARK: - Strict Navigation URL Validation (`validate_wk_navigation_url`)

    /// Permits ONLY `https://` and `http://` URLs with a valid non-empty host.
    /// Rejects `javascript:`, `file:`, `data:`, `blob:`, control chars, or empty strings fail-closed.
    public static func validateWKNavigationURL(_ raw: String) -> URL? {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        for scalar in trimmed.unicodeScalars {
            if scalar.value < 0x20 || scalar.value == 0x7F {
                return nil
            }
        }
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              scheme == "https" || scheme == "http",
              let host = components.host?.trimmingCharacters(in: .whitespacesAndNewlines),
              !host.isEmpty,
              let url = components.url
        else {
            return nil
        }
        return url
    }

    // MARK: - Strict Automation App ID Validation (`api_automation_apps.rs:13-28`)

    /// Validates `app_id`: 1...128 ASCII chars in `[a-zA-Z0-9_-]`, forbidding `"installed"` (case-insensitive).
    public static func validateAutomationAppID(_ id: String) -> Bool {
        let count = id.utf8.count
        guard count >= 1 && count <= 128 else { return false }
        if id.lowercased() == "installed" {
            return false
        }
        return id.unicodeScalars.allSatisfy { scalar in
            (scalar.value >= 0x30 && scalar.value <= 0x39) // 0-9
                || (scalar.value >= 0x41 && scalar.value <= 0x5A) // A-Z
                || (scalar.value >= 0x61 && scalar.value <= 0x7A) // a-z
                || scalar == "_"
                || scalar == "-"
        }
    }

    // MARK: - Canonical Manifest SHA-256 Digest

    public static func computeManifestSHA256(_ manifestJSON: String) -> String {
        let digest = SHA256.hash(data: Data(manifestJSON.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    public static func computeManifestSHA256(_ app: FabushiAutomationScriptApp) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(app)
        let digest = SHA256.hash(data: data)
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Fail-Closed Secret-Leak Preflight (`validate_script_app_safe_for_publish`)

    public static func isSensitiveParameterName(_ name: String) -> Bool {
        let lower = name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let sensitiveKeywords = [
            "password", "passwd", "pwd", "secret", "token", "api_key", "apikey",
            "access_key", "private_key", "auth", "cookie", "session", "otp", "totp",
            "cvv", "cvc", "pin", "passphrase", "bearer",
        ]
        return sensitiveKeywords.contains { lower.contains($0) }
    }

    public static func scanTextForSecretLeaks(
        _ text: String,
        fieldPath: String
    ) -> [FabushiBrowserSecretLeakFinding] {
        guard !text.isEmpty else { return [] }
        // Strip safe template placeholders (`{{param}}`) before scanning literal tokens
        let withoutPlaceholders = text.replacingOccurrences(
            of: #"\{\{[a-zA-Z0-9_.-]+\}\}"#,
            with: "",
            options: .regularExpression
        )
        guard !withoutPlaceholders.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return []
        }

        var findings: [FabushiBrowserSecretLeakFinding] = []
        let patterns: [(rule: String, regex: String, summary: String)] = [
            (
                "private_key",
                #"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"#,
                "PEM private key block detected"
            ),
            (
                "jwt_token",
                #"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"#,
                "JWT bearer token detected"
            ),
            (
                "bearer_header",
                #"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{12,}"#,
                "Authorization Bearer credential detected"
            ),
            (
                "api_token_prefix",
                #"\b(?:sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{12,}|AIza[0-9A-Za-z_-]{20,}|AKIA[0-9A-Z]{16})\b"#,
                "Provider API/OAuth token detected"
            ),
            (
                "sensitive_cookie",
                #"(?i)(?:\bset-cookie\s*:|\b(?:ct0|auth_token|session_id|sessionid|connect\.sid)\s*=\s*[^\s;]{6,})"#,
                "Session/auth cookie value detected"
            ),
            (
                "hardcoded_password",
                #"(?i)\b(?:password|passwd|pwd|client_secret|api_secret|access_token|refresh_token)\s*[:=]\s*['"]?[^\s'",;]{4,}"#,
                "Hardcoded password or secret assignment detected"
            ),
        ]

        for entry in patterns {
            if withoutPlaceholders.range(of: entry.regex, options: .regularExpression) != nil {
                findings.append(
                    FabushiBrowserSecretLeakFinding(
                        fieldPath: fieldPath,
                        rule: entry.rule,
                        summary: entry.summary
                    )
                )
            }
        }
        return findings
    }

    public static func validateScriptAppSafeForPublish(_ app: FabushiAutomationScriptApp) throws {
        if app.schemaVersion == 0 {
            throw FabushiBrowserPreflightError.invalidScript("schema_version must be >= 1")
        }
        if !validateAutomationAppID(app.appId) {
            throw FabushiBrowserPreflightError.invalidScript("Invalid app_id '\(app.appId)'")
        }
        if app.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw FabushiBrowserPreflightError.invalidScript("name cannot be empty")
        }
        if app.version.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            throw FabushiBrowserPreflightError.invalidScript("version cannot be empty")
        }
        if app.steps.isEmpty {
            throw FabushiBrowserPreflightError.invalidScript("steps cannot be empty")
        }
        let validSlug = !app.slug.isEmpty && app.slug.unicodeScalars.allSatisfy { scalar in
            (scalar.value >= 0x61 && scalar.value <= 0x7A)
                || (scalar.value >= 0x30 && scalar.value <= 0x39)
                || scalar == "-" || scalar == "_"
        }
        if !validSlug {
            throw FabushiBrowserPreflightError.invalidScript("Invalid slug '\(app.slug)': expected [a-z0-9_-]")
        }

        var findings: [FabushiBrowserSecretLeakFinding] = []
        var seenParamNames = Set<String>()
        for (idx, param) in app.parameters.enumerated() {
            let trimmedName = param.name.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmedName.isEmpty {
                throw FabushiBrowserPreflightError.invalidScript("parameters[\(idx)].name cannot be empty")
            }
            if !seenParamNames.insert(trimmedName).inserted {
                throw FabushiBrowserPreflightError.invalidScript("Duplicate parameter name '\(trimmedName)'")
            }
            if param.isSecret, let defaultVal = param.defaultValue, !defaultVal.isEmpty {
                findings.append(
                    FabushiBrowserSecretLeakFinding(
                        fieldPath: "app.parameters[\(idx)].default_value",
                        rule: "secret_param_default",
                        summary: "Secret parameter '\(trimmedName)' must not carry a default_value"
                    )
                )
            }
            if isSensitiveParameterName(trimmedName) {
                if !param.isSecret || (param.defaultValue?.isEmpty == false) {
                    findings.append(
                        FabushiBrowserSecretLeakFinding(
                            fieldPath: "app.parameters[\(idx)]",
                            rule: "sensitive_param_unmarked",
                            summary: "Sensitive parameter '\(trimmedName)' must be marked is_secret=true with no default_value"
                        )
                    )
                }
            }
            if let defaultVal = param.defaultValue {
                findings.append(contentsOf: scanTextForSecretLeaks(defaultVal, fieldPath: "app.parameters[\(idx)].default_value"))
            }
            findings.append(contentsOf: scanTextForSecretLeaks(param.description, fieldPath: "app.parameters[\(idx)].description"))
        }

        findings.append(contentsOf: scanTextForSecretLeaks(app.name, fieldPath: "app.name"))
        findings.append(contentsOf: scanTextForSecretLeaks(app.description, fieldPath: "app.description"))
        if let traceSummary = app.sourceTraceSummary {
            findings.append(contentsOf: scanTextForSecretLeaks(traceSummary, fieldPath: "app.source_trace_summary"))
        }

        for (idx, step) in app.steps.enumerated() {
            switch step {
            case .navigate(let urlTemplate):
                findings.append(contentsOf: scanTextForSecretLeaks(urlTemplate, fieldPath: "app.steps[\(idx)].url_template"))
                let substituted = urlTemplate.replacingOccurrences(
                    of: #"\{\{[a-zA-Z0-9_.-]+\}\}"#,
                    with: "example",
                    options: .regularExpression
                )
                guard let url = validateWKNavigationURL(substituted),
                      let host = url.host?.lowercased()
                else {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] invalid navigate url_template")
                }
                if host == "169.254.169.254" || host == "metadata.google.internal" {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] blocked cloud metadata URL")
                }
            case .waitFor(let locator, let timeoutMs):
                if locator.isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] locator cannot be empty")
                }
                if timeoutMs == 0 {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] timeout_ms must be > 0")
                }
            case .click(let locator), .assertVisible(let locator):
                if locator.isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] locator cannot be empty")
                }
            case .fill(let locator, let valueTemplate, let secretVaultKey):
                if locator.isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] locator cannot be empty")
                }
                findings.append(contentsOf: scanTextForSecretLeaks(valueTemplate, fieldPath: "app.steps[\(idx)].value_template"))
                let targetHint = [locator.nameAttr, locator.ariaLabel, locator.testId, locator.cssSelector]
                    .compactMap { $0 }
                    .joined(separator: " ")
                let isPlaceholderOnly = valueTemplate.range(
                    of: #"^\s*\{\{[a-zA-Z0-9_.-]+\}\}\s*$"#,
                    options: .regularExpression
                ) != nil
                if isSensitiveParameterName(targetHint) && !isPlaceholderOnly && (secretVaultKey?.isEmpty != false) {
                    findings.append(
                        FabushiBrowserSecretLeakFinding(
                            fieldPath: "app.steps[\(idx)].value_template",
                            rule: "sensitive_field_literal",
                            summary: "Literal value in sensitive field must be parameterized or bound to Vault"
                        )
                    )
                }
            case .assertTextContains(let locator, let expectedTemplate):
                if locator.isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] locator cannot be empty")
                }
                findings.append(contentsOf: scanTextForSecretLeaks(expectedTemplate, fieldPath: "app.steps[\(idx)].expected_template"))
            case .extractText(let locator, let outputKey):
                if locator.isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] locator cannot be empty")
                }
                if outputKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] output_key cannot be empty")
                }
            case .pressKey(let key):
                if key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    throw FabushiBrowserPreflightError.invalidScript("steps[\(idx)] key cannot be empty")
                }
            case .scroll:
                break
            case .humanCheckpoint(let reason):
                findings.append(contentsOf: scanTextForSecretLeaks(reason, fieldPath: "app.steps[\(idx)].reason"))
            }
        }

        if !findings.isEmpty {
            throw FabushiBrowserPreflightError.secretLeakDetected(findings)
        }
    }
}

// MARK: - Cookie Parser / Exporter (CDP JSON, EditThisCookie JSON, Netscape `cookies.txt`)

public enum FabushiBrowserCookieCodec: Sendable {
    public struct VaultProfileBundle: Codable, Sendable, Equatable {
        public let schemaVersion: Int
        public let profileKey: String
        public let exportedAtMs: Int64
        public let cookies: [FabushiBrowserCookieRecord]

        public init(
            schemaVersion: Int = 1,
            profileKey: String,
            exportedAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000),
            cookies: [FabushiBrowserCookieRecord]
        ) {
            self.schemaVersion = schemaVersion
            self.profileKey = profileKey
            self.exportedAtMs = exportedAtMs
            self.cookies = cookies
        }

        enum CodingKeys: String, CodingKey {
            case schemaVersion = "schema_version"
            case profileKey = "profile_key"
            case exportedAtMs = "exported_at_ms"
            case cookies
        }
    }

    /// Parses CDP JSON (`{"cookies":[...]}` or `[...]`), EditThisCookie JSON (`expirationDate`),
    /// or Netscape `cookies.txt`, enforcing the 16 MiB local parse limit.
    public static func parseCookies(from rawText: String) throws -> [FabushiBrowserCookieRecord] {
        let byteCount = rawText.utf8.count
        guard byteCount <= FabushiBrowserSecurity.maxCookiesJSONBytes else {
            throw FabushiBrowserPreflightError.cookiePayloadTooLarge(
                actualBytes: byteCount,
                maxBytes: FabushiBrowserSecurity.maxCookiesJSONBytes
            )
        }
        let trimmed = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw FabushiBrowserPreflightError.invalidCookiePayload("Cookie input is empty.")
        }

        if trimmed.hasPrefix("[") || trimmed.hasPrefix("{") {
            return try parseJSONCookies(Data(trimmed.utf8))
        }
        return try parseNetscapeCookies(trimmed)
    }

    private static func parseJSONCookies(_ data: Data) throws -> [FabushiBrowserCookieRecord] {
        let jsonObject = try JSONSerialization.jsonObject(with: data, options: [])
        let rawList: [[String: Any]]
        if let arr = jsonObject as? [[String: Any]] {
            rawList = arr
        } else if let dict = jsonObject as? [String: Any],
                  let arr = dict["cookies"] as? [[String: Any]]
        {
            rawList = arr
        } else {
            throw FabushiBrowserPreflightError.invalidCookiePayload("Expected JSON cookie array or object with 'cookies' array.")
        }

        var results: [FabushiBrowserCookieRecord] = []
        results.reserveCapacity(rawList.count)
        for item in rawList {
            guard let name = (item["name"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !name.isEmpty,
                  let value = item["value"] as? String,
                  let domain = (item["domain"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !domain.isEmpty
            else {
                continue
            }
            let path = (item["path"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
            let normalizedPath = (path?.isEmpty == false) ? path! : "/"
            let expires = (item["expires"] as? NSNumber)?.doubleValue
                ?? (item["expirationDate"] as? NSNumber)?.doubleValue
            let secure = (item["secure"] as? Bool) ?? false
            let httpOnly = (item["httpOnly"] as? Bool) ?? false
            results.append(
                FabushiBrowserCookieRecord(
                    name: name,
                    value: value,
                    domain: domain,
                    path: normalizedPath,
                    expiresUnixSeconds: expires,
                    secure: secure,
                    httpOnly: httpOnly
                )
            )
        }
        guard !results.isEmpty else {
            throw FabushiBrowserPreflightError.invalidCookiePayload("No valid cookies found in JSON payload.")
        }
        return results
    }

    private static func parseNetscapeCookies(_ raw: String) throws -> [FabushiBrowserCookieRecord] {
        var results: [FabushiBrowserCookieRecord] = []
        for rawLine in raw.components(separatedBy: .newlines) {
            let line = rawLine.trimmingCharacters(in: .whitespacesAndNewlines)
            if line.isEmpty { continue }
            var effectiveLine = line
            var httpOnly = false
            if effectiveLine.hasPrefix("#HttpOnly_") {
                httpOnly = true
                effectiveLine = String(effectiveLine.dropFirst("#HttpOnly_".count))
            } else if effectiveLine.hasPrefix("#") {
                continue
            }
            let fields = effectiveLine.components(separatedBy: "\t")
            guard fields.count >= 7 else { continue }
            let domain = fields[0].trimmingCharacters(in: .whitespacesAndNewlines)
            let path = fields[2].isEmpty ? "/" : fields[2]
            let secure = fields[3].uppercased() == "TRUE"
            let expiresRaw = Double(fields[4])
            let expires = (expiresRaw ?? 0) > 0 ? expiresRaw : nil
            let name = fields[5].trimmingCharacters(in: .whitespacesAndNewlines)
            let value = fields[6]
            guard !domain.isEmpty, !name.isEmpty else { continue }
            results.append(
                FabushiBrowserCookieRecord(
                    name: name,
                    value: value,
                    domain: domain,
                    path: path,
                    expiresUnixSeconds: expires,
                    secure: secure,
                    httpOnly: httpOnly
                )
            )
        }
        guard !results.isEmpty else {
            throw FabushiBrowserPreflightError.invalidCookiePayload("No valid Netscape cookies.txt entries found.")
        }
        return results
    }

    public static func exportCDPJSON(_ cookies: [FabushiBrowserCookieRecord]) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(cookies)
        return String(decoding: data, as: UTF8.self)
    }

    /// Encodes a profile bundle for E2EE Vault cloud sync, strictly enforcing the `<= 512 KiB` cap.
    public static func encodeVaultBundle(
        profileKey: String,
        cookies: [FabushiBrowserCookieRecord]
    ) throws -> String {
        let bundle = VaultProfileBundle(profileKey: profileKey, cookies: cookies)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(bundle)
        guard data.count <= FabushiBrowserSecurity.maxVaultBundleBytes else {
            throw FabushiBrowserPreflightError.vaultBundleTooLarge(
                actualBytes: data.count,
                maxBytes: FabushiBrowserSecurity.maxVaultBundleBytes
            )
        }
        return String(decoding: data, as: UTF8.self)
    }

    public static func decodeVaultBundle(_ jsonString: String) throws -> VaultProfileBundle {
        let data = Data(jsonString.utf8)
        guard data.count <= FabushiBrowserSecurity.maxVaultBundleBytes else {
            throw FabushiBrowserPreflightError.vaultBundleTooLarge(
                actualBytes: data.count,
                maxBytes: FabushiBrowserSecurity.maxVaultBundleBytes
            )
        }
        return try JSONDecoder().decode(VaultProfileBundle.self, from: data)
    }
}
