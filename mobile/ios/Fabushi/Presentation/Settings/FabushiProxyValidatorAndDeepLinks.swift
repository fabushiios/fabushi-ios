import Darwin
import Foundation

// MARK: - D10-13 Strict Proxy Models, Validator & RFC-3986 `tg://` Deep Links

/// Canonical proxy kind matching Rust `ClientProxyType` (PascalCase `"Direct"`, `"Http"`, `"Socks5"`)
/// plus Telegram `Mtproto`.
public enum FabushiProxyKind: String, Codable, CaseIterable, Sendable, Equatable, Identifiable {
    case direct = "Direct"
    case http = "Http"
    case socks5 = "Socks5"
    case mtproto = "Mtproto"

    public var id: String { rawValue }

    public var isRustHostSupported: Bool {
        switch self {
        case .direct, .http, .socks5:
            return true
        case .mtproto:
            return false
        }
    }

    public static func parse(_ raw: String) -> FabushiProxyKind? {
        switch raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "direct", "none":
            return .direct
        case "http", "https":
            return .http
        case "socks5", "socks5h", "socks":
            return .socks5
        case "mtproto", "mtproxy":
            return .mtproto
        default:
            return nil
        }
    }

    public func localizedTitle(locale: FabushiLocale) -> String {
        switch self {
        case .direct:
            return FabushiPlatformStrings.text(.settingsProxyKindDirect, locale: locale)
        case .http:
            return FabushiPlatformStrings.text(.settingsProxyKindHttp, locale: locale)
        case .socks5:
            return FabushiPlatformStrings.text(.settingsProxyKindSocks5, locale: locale)
        case .mtproto:
            return FabushiPlatformStrings.text(.settingsProxyKindMtproto, locale: locale)
        }
    }
}

public struct FabushiProxyRecord: Codable, Identifiable, Sendable, Equatable {
    public let id: String
    public let kind: FabushiProxyKind
    public let host: String
    public let port: UInt16
    public let username: String?
    public let password: String?
    public let mtprotoSecret: String
    public let fallbackToDirect: Bool
    public var active: Bool
    public var lastLatencyMs: UInt32?

    public init(
        id: String = "proxy_\(UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased())",
        kind: FabushiProxyKind,
        host: String,
        port: UInt16,
        username: String? = nil,
        password: String? = nil,
        mtprotoSecret: String = "",
        fallbackToDirect: Bool = false,
        active: Bool = false,
        lastLatencyMs: UInt32? = nil
    ) {
        self.id = id
        self.kind = kind
        self.host = host
        self.port = port
        self.username = username
        self.password = password
        self.mtprotoSecret = mtprotoSecret
        self.fallbackToDirect = fallbackToDirect
        self.active = active
        self.lastLatencyMs = lastLatencyMs
    }

    public var displayEndpoint: String {
        if kind == .direct {
            return "Direct"
        }
        let formattedHost = host.contains(":") && !host.hasPrefix("[") ? "[\(host)]" : host
        return "\(formattedHost):\(port)"
    }
}

public enum FabushiProxyValidationError: LocalizedError, Equatable, Sendable {
    case emptyHost
    case invalidHostWhitespaceOrControl
    case invalidHostUrlCharacters
    case invalidIPv6Host
    case invalidPort(Int)
    case passwordWithoutUsername
    case usernameTooLong
    case usernameInvalidCharacters
    case passwordTooLong
    case passwordInvalidCharacters
    case mtprotoSecretTooLong
    case mtprotoSecretInvalidCharacters
    case unsupportedLinkScheme
    case malformedDeepLink

    public var errorDescription: String? {
        switch self {
        case .emptyHost:
            return "Proxy host cannot be empty."
        case .invalidHostWhitespaceOrControl:
            return "Proxy host contains invalid control or whitespace characters."
        case .invalidHostUrlCharacters:
            return "Proxy host contains invalid URL metacharacters."
        case .invalidIPv6Host:
            return "Proxy host contains ':' but is not a valid IPv6 address."
        case .invalidPort(let port):
            return "Proxy port \(port) must be between 1 and 65535."
        case .passwordWithoutUsername:
            return "Proxy password specified without username."
        case .usernameTooLong:
            return "Proxy username exceeds 255 UTF-8 bytes."
        case .usernameInvalidCharacters:
            return "Proxy username contains invalid control characters or ':'."
        case .passwordTooLong:
            return "Proxy password exceeds 255 UTF-8 bytes."
        case .passwordInvalidCharacters:
            return "Proxy password contains invalid control characters."
        case .mtprotoSecretTooLong:
            return "MTProto proxy secret exceeds 1024 bytes."
        case .mtprotoSecretInvalidCharacters:
            return "MTProto proxy secret must contain only hex or base64url characters."
        case .unsupportedLinkScheme:
            return "Unsupported proxy link scheme. Expected tg://proxy, tg://socks, http://, or socks5://."
        case .malformedDeepLink:
            return "Malformed proxy share link."
        }
    }
}

/// Strict pre-mutation validator mirroring `ClientProxyConfig::validate_record` (`D10-13`).
public enum FabushiProxyValidator: Sendable {
    private static let forbiddenHostChars: Set<Character> = [
        "@", "/", "\\", "?", "#", "[", "]", "%", ",", ";", "&", "="
    ]

    public static func isValidIPv6Address(_ candidate: String) -> Bool {
        guard !candidate.isEmpty, candidate.contains(":") else { return false }
        var addr = in6_addr()
        return candidate.withCString { cStr in
            inet_pton(AF_INET6, cStr, &addr) == 1
        }
    }

    /// Strips surrounding `[...]` ONLY when the interior is a valid IPv6 literal (e.g. `[::1]` -> `::1`).
    public static func stripIPv6Brackets(_ rawHost: String) -> String {
        guard rawHost.count >= 4, rawHost.first == "[", rawHost.last == "]" else {
            return rawHost
        }
        let inner = String(rawHost.dropFirst().dropLast())
        return isValidIPv6Address(inner) ? inner : rawHost
    }

    public static func validateHost(_ rawHost: String) throws -> String {
        guard !rawHost.isEmpty else {
            throw FabushiProxyValidationError.emptyHost
        }
        if containsAsciiControlOrWhitespace(rawHost) {
            throw FabushiProxyValidationError.invalidHostWhitespaceOrControl
        }

        let normalized = stripIPv6Brackets(rawHost)
        guard !normalized.isEmpty else {
            throw FabushiProxyValidationError.emptyHost
        }
        if normalized.contains("://") || normalized.contains(where: { forbiddenHostChars.contains($0) }) {
            throw FabushiProxyValidationError.invalidHostUrlCharacters
        }
        if normalized.contains(":") && !isValidIPv6Address(normalized) {
            throw FabushiProxyValidationError.invalidIPv6Host
        }
        return normalized
    }

    public static func validatePort(_ port: Int) throws -> UInt16 {
        guard (1...65535).contains(port) else {
            throw FabushiProxyValidationError.invalidPort(port)
        }
        return UInt16(port)
    }

    public static func validateCredentials(username: String?, password: String?) throws {
        let effectiveUser = (username?.isEmpty == false) ? username : nil
        let effectivePass = (password?.isEmpty == false) ? password : nil

        if effectivePass != nil && effectiveUser == nil {
            throw FabushiProxyValidationError.passwordWithoutUsername
        }
        if let u = effectiveUser {
            if u.utf8.count > 255 {
                throw FabushiProxyValidationError.usernameTooLong
            }
            if containsControlCharacter(u) || u.contains(":") {
                throw FabushiProxyValidationError.usernameInvalidCharacters
            }
        }
        if let p = effectivePass {
            if p.utf8.count > 255 {
                throw FabushiProxyValidationError.passwordTooLong
            }
            if containsControlCharacter(p) {
                throw FabushiProxyValidationError.passwordInvalidCharacters
            }
        }
    }

    /// Validates an MTProto secret strictly (<= 1024 bytes, no control/whitespace,
    /// hex or base64url `[A-Za-z0-9_-]` + optional trailing `=` or `==`; NEVER splits `:`).
    public static func validateMTProtoSecret(_ secret: String) throws {
        if secret.isEmpty {
            return
        }
        if secret.utf8.count > 1024 {
            throw FabushiProxyValidationError.mtprotoSecretTooLong
        }
        if containsControlOrWhitespace(secret) {
            throw FabushiProxyValidationError.mtprotoSecretInvalidCharacters
        }

        let core: Substring
        if secret.hasSuffix("==") {
            core = secret.dropLast(2)
        } else if secret.hasSuffix("=") {
            core = secret.dropLast(1)
        } else {
            core = secret[...]
        }

        guard !core.isEmpty else {
            throw FabushiProxyValidationError.mtprotoSecretInvalidCharacters
        }
        for byte in core.utf8 {
            let isAlphaNum = (byte >= 0x30 && byte <= 0x39)
                || (byte >= 0x41 && byte <= 0x5A)
                || (byte >= 0x61 && byte <= 0x7A)
            let isUrlSafe = byte == 0x2D || byte == 0x5F // '-' or '_'
            if !isAlphaNum && !isUrlSafe {
                throw FabushiProxyValidationError.mtprotoSecretInvalidCharacters
            }
        }
    }

    /// Validates an entire proxy record pre-mutation and returns the normalized record (IPv6 brackets stripped).
    public static func validate(_ record: FabushiProxyRecord) throws -> FabushiProxyRecord {
        switch record.kind {
        case .direct:
            return record
        case .mtproto:
            let cleanHost = try validateHost(record.host)
            let cleanPort = try validatePort(Int(record.port))
            try validateMTProtoSecret(record.mtprotoSecret)
            return FabushiProxyRecord(
                id: record.id,
                kind: .mtproto,
                host: cleanHost,
                port: cleanPort,
                username: nil,
                password: nil,
                mtprotoSecret: record.mtprotoSecret,
                fallbackToDirect: record.fallbackToDirect,
                active: record.active,
                lastLatencyMs: record.lastLatencyMs
            )
        case .http, .socks5:
            let cleanHost = try validateHost(record.host)
            let cleanPort = try validatePort(Int(record.port))
            let normalizedUser = (record.username?.isEmpty == false) ? record.username : nil
            let normalizedPass = (record.password?.isEmpty == false) ? record.password : nil
            try validateCredentials(username: normalizedUser, password: normalizedPass)
            return FabushiProxyRecord(
                id: record.id,
                kind: record.kind,
                host: cleanHost,
                port: cleanPort,
                username: normalizedUser,
                password: normalizedPass,
                mtprotoSecret: "",
                fallbackToDirect: record.fallbackToDirect,
                active: record.active,
                lastLatencyMs: record.lastLatencyMs
            )
        }
    }

    private static func containsAsciiControlOrWhitespace(_ text: String) -> Bool {
        for scalar in text.unicodeScalars {
            if scalar.value <= 0x1F || scalar.value == 0x7F || scalar.properties.isWhitespace {
                return true
            }
        }
        return false
    }

    private static func containsControlCharacter(_ text: String) -> Bool {
        for scalar in text.unicodeScalars {
            if scalar.value <= 0x1F || scalar.value == 0x7F || scalar.properties.generalCategory == .control {
                return true
            }
        }
        return false
    }

    private static func containsControlOrWhitespace(_ text: String) -> Bool {
        for scalar in text.unicodeScalars {
            if scalar.value <= 0x1F
                || scalar.value == 0x7F
                || scalar.properties.generalCategory == .control
                || scalar.properties.isWhitespace
            {
                return true
            }
        }
        return false
    }
}

/// RFC-3986 percent-encoding/decoding and `tg://proxy` / `tg://socks` deep-link exporter & importer
/// matching `session_persistence.rs` and `deep_links_router.rs` byte-for-byte (`D10-13`).
public enum FabushiProxyDeepLinks: Sendable {
    private static let hexDigits: [UInt8] = Array("0123456789ABCDEF".utf8)
    public static let defaultMtprotoPlaceholderSecret = "ee000000000000000000000000000001"

    /// Strictly RFC-3986 percent-encodes UTF-8 bytes (preserving only unreserved `A-Z a-z 0-9 - . _ ~`).
    public static func percentEncodeRFC3986(_ input: String) -> String {
        var out: [UInt8] = []
        out.reserveCapacity(input.utf8.count * 3)
        for byte in input.utf8 {
            let isUnreserved = (byte >= 0x30 && byte <= 0x39)
                || (byte >= 0x41 && byte <= 0x5A)
                || (byte >= 0x61 && byte <= 0x7A)
                || byte == 0x2D || byte == 0x2E || byte == 0x5F || byte == 0x7E
            if isUnreserved {
                out.append(byte)
            } else {
                out.append(0x25) // '%'
                out.append(hexDigits[Int(byte >> 4)])
                out.append(hexDigits[Int(byte & 0x0F)])
            }
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// Decodes percent-encoded UTF-8 sequences and `+` -> space cleanly without panicking on malformed `%`.
    public static func percentDecode(_ input: String) -> String {
        let bytes = Array(input.utf8)
        var out: [UInt8] = []
        out.reserveCapacity(bytes.count)
        var index = 0
        while index < bytes.count {
            if bytes[index] == 0x25, index + 2 < bytes.count,
               let hi = hexNibble(bytes[index + 1]),
               let lo = hexNibble(bytes[index + 2])
            {
                out.append((hi << 4) | lo)
                index += 3
                continue
            } else if bytes[index] == 0x2B { // '+'
                out.append(0x20)
                index += 1
                continue
            }
            out.append(bytes[index])
            index += 1
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// Exports a canonical share link (`Direct` -> `""`, `MTProto` -> `tg://proxy`,
    /// `SOCKS5` -> `tg://socks`, `HTTP` -> `http://[ipv6_or_host]:port`).
    public static func exportLink(for record: FabushiProxyRecord) -> String {
        switch record.kind {
        case .direct:
            return ""
        case .mtproto:
            let trimmedSecret = record.mtprotoSecret.trimmingCharacters(in: .whitespacesAndNewlines)
            let effectiveSecret = trimmedSecret.isEmpty ? defaultMtprotoPlaceholderSecret : trimmedSecret
            return "tg://proxy?server=\(percentEncodeRFC3986(record.host.trimmingCharacters(in: .whitespacesAndNewlines)))&port=\(record.port)&secret=\(percentEncodeRFC3986(effectiveSecret))"
        case .socks5:
            var link = "tg://socks?server=\(percentEncodeRFC3986(record.host.trimmingCharacters(in: .whitespacesAndNewlines)))&port=\(record.port)"
            if let user = record.username, !user.isEmpty {
                link += "&user=\(percentEncodeRFC3986(user))"
            }
            if let pass = record.password, !pass.isEmpty {
                link += "&pass=\(percentEncodeRFC3986(pass))"
            }
            return link
        case .http:
            let trimmedHost = record.host.trimmingCharacters(in: .whitespacesAndNewlines)
            let formattedHost = trimmedHost.contains(":") && !trimmedHost.hasPrefix("[")
                ? "[\(trimmedHost)]"
                : trimmedHost
            return "http://\(formattedHost):\(record.port)"
        }
    }

    /// Parses and strictly validates `tg://proxy?...`, `tg://socks?...`, `http://...`, `https://...`,
    /// `socks5://...`, or `socks5h://...` links.
    public static func parseImportLink(_ rawLink: String) throws -> FabushiProxyRecord {
        let trimmed = rawLink.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw FabushiProxyValidationError.malformedDeepLink
        }

        let lower = trimmed.lowercased()
        if lower.hasPrefix("tg://proxy") || lower.hasPrefix("tg://socks") {
            let isSocks = lower.hasPrefix("tg://socks")
            guard let queryStart = trimmed.firstIndex(of: "?") else {
                throw FabushiProxyValidationError.malformedDeepLink
            }
            var querySlice = trimmed[trimmed.index(after: queryStart)...]
            if let fragmentStart = querySlice.firstIndex(of: "#") {
                querySlice = querySlice[..<fragmentStart]
            }

            var params: [String: String] = [:]
            for pair in querySlice.split(separator: "&", omittingEmptySubsequences: true) {
                if let eq = pair.firstIndex(of: "=") {
                    let key = String(pair[..<eq]).lowercased()
                    let rawVal = String(pair[pair.index(after: eq)...])
                    params[key] = percentDecode(rawVal)
                }
            }

            guard let rawServer = params["server"],
                  let rawPort = params["port"],
                  let portInt = Int(rawPort.trimmingCharacters(in: .whitespacesAndNewlines))
            else {
                throw FabushiProxyValidationError.malformedDeepLink
            }

            let cleanHost = try FabushiProxyValidator.validateHost(rawServer)
            let cleanPort = try FabushiProxyValidator.validatePort(portInt)

            if isSocks {
                let user = params["user"]
                let pass = params["pass"]
                let candidate = FabushiProxyRecord(
                    kind: .socks5,
                    host: cleanHost,
                    port: cleanPort,
                    username: (user?.isEmpty == false) ? user : nil,
                    password: (pass?.isEmpty == false) ? pass : nil
                )
                return try FabushiProxyValidator.validate(candidate)
            } else {
                let secret = params["secret"] ?? ""
                let candidate = FabushiProxyRecord(
                    kind: .mtproto,
                    host: cleanHost,
                    port: cleanPort,
                    mtprotoSecret: secret
                )
                return try FabushiProxyValidator.validate(candidate)
            }
        }

        // Standard URL formats: http://, https://, socks5://, socks5h://
        guard let components = URLComponents(string: trimmed),
              let scheme = components.scheme?.lowercased(),
              let rawHost = components.host
        else {
            throw FabushiProxyValidationError.unsupportedLinkScheme
        }

        let kind: FabushiProxyKind
        switch scheme {
        case "http", "https":
            kind = .http
        case "socks5", "socks5h":
            kind = .socks5
        default:
            throw FabushiProxyValidationError.unsupportedLinkScheme
        }

        guard let portInt = components.port else {
            throw FabushiProxyValidationError.invalidPort(0)
        }

        let candidate = FabushiProxyRecord(
            kind: kind,
            host: rawHost,
            port: try FabushiProxyValidator.validatePort(portInt),
            username: components.user,
            password: components.password
        )
        return try FabushiProxyValidator.validate(candidate)
    }

    private static func hexNibble(_ byte: UInt8) -> UInt8? {
        switch byte {
        case 0x30...0x39: return byte - 0x30
        case 0x41...0x46: return byte - 0x41 + 10
        case 0x61...0x66: return byte - 0x61 + 10
        default: return nil
        }
    }
}
