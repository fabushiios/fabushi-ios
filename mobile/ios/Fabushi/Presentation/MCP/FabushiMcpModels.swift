import Foundation

// MARK: - Official OAuth 2.1 MCP Catalog (14 Scopes & 31 Tools — `v1.6.15` `official_mcp.rs`)

public struct FabushiMcpOfficialToolSpec: Sendable, Equatable, Identifiable {
    public let name: String
    public let scope: String

    public var id: String { name }

    public init(name: String, scope: String) {
        self.name = name
        self.scope = scope
    }
}

public enum FabushiMcpOfficialCatalog: Sendable {
    public static let protocolVersion = "2025-11-25"
    public static let serverName = "fabushi-official-mcp"

    /// Exact 14 canonical `MCP_SUPPORTED_SCOPES` (`mcp_oauth.rs:L27-L42`).
    public static let supportedScopes: [String] = [
        "devices:read",
        "devices:connect",
        "screen:view",
        "input:control",
        "terminal:exec",
        "files:read",
        "files:write",
        "assist:manage",
        "power:control",
        "messaging:read",
        "messaging:write",
        "chats:read",
        "miniapp:control",
        "tasks:manage",
    ]

    /// Complete 31 Official MCP tools shipped in locked `v1.6.15` (`official_mcp.rs:L744-L769, L2038-L2356`).
    public static let officialTools: [FabushiMcpOfficialToolSpec] = [
        // devices:read (2)
        FabushiMcpOfficialToolSpec(name: "list_devices", scope: "devices:read"),
        FabushiMcpOfficialToolSpec(name: "get_device_status", scope: "devices:read"),
        // devices:connect (1)
        FabushiMcpOfficialToolSpec(name: "request_human_takeover", scope: "devices:connect"),
        // screen:view (2)
        FabushiMcpOfficialToolSpec(name: "capture_screen", scope: "screen:view"),
        FabushiMcpOfficialToolSpec(name: "get_accessibility_tree", scope: "screen:view"),
        // input:control (4)
        FabushiMcpOfficialToolSpec(name: "semantic_ui_action", scope: "input:control"),
        FabushiMcpOfficialToolSpec(name: "mouse_action", scope: "input:control"),
        FabushiMcpOfficialToolSpec(name: "keyboard_type", scope: "input:control"),
        FabushiMcpOfficialToolSpec(name: "keyboard_hotkey", scope: "input:control"),
        // terminal:exec (6)
        FabushiMcpOfficialToolSpec(name: "terminal_list", scope: "terminal:exec"),
        FabushiMcpOfficialToolSpec(name: "terminal_open", scope: "terminal:exec"),
        FabushiMcpOfficialToolSpec(name: "terminal_exec", scope: "terminal:exec"),
        FabushiMcpOfficialToolSpec(name: "terminal_send", scope: "terminal:exec"),
        FabushiMcpOfficialToolSpec(name: "terminal_read", scope: "terminal:exec"),
        FabushiMcpOfficialToolSpec(name: "terminal_kill", scope: "terminal:exec"),
        // files:read (3)
        FabushiMcpOfficialToolSpec(name: "file_list", scope: "files:read"),
        FabushiMcpOfficialToolSpec(name: "file_read", scope: "files:read"),
        FabushiMcpOfficialToolSpec(name: "clipboard_get", scope: "files:read"),
        // files:write (2)
        FabushiMcpOfficialToolSpec(name: "file_write", scope: "files:write"),
        FabushiMcpOfficialToolSpec(name: "clipboard_set", scope: "files:write"),
        // assist:manage (2)
        FabushiMcpOfficialToolSpec(name: "assist_get_info", scope: "assist:manage"),
        FabushiMcpOfficialToolSpec(name: "assist_manage", scope: "assist:manage"),
        // power:control (1)
        FabushiMcpOfficialToolSpec(name: "device_power_action", scope: "power:control"),
        // chats:read (2)
        FabushiMcpOfficialToolSpec(name: "list_dialogs", scope: "chats:read"),
        FabushiMcpOfficialToolSpec(name: "list_bots", scope: "chats:read"),
        // messaging:read (2)
        FabushiMcpOfficialToolSpec(name: "get_chat_history", scope: "messaging:read"),
        FabushiMcpOfficialToolSpec(name: "search_messages", scope: "messaging:read"),
        // messaging:write (1)
        FabushiMcpOfficialToolSpec(name: "send_message", scope: "messaging:write"),
        // tasks:manage (2)
        FabushiMcpOfficialToolSpec(name: "list_tasks", scope: "tasks:manage"),
        FabushiMcpOfficialToolSpec(name: "create_task", scope: "tasks:manage"),
        // miniapp:control (1)
        FabushiMcpOfficialToolSpec(name: "miniapp_list", scope: "miniapp:control"),
    ]

    public static func requiredScope(for toolName: String) -> String? {
        officialTools.first(where: { $0.name == toolName })?.scope
    }
}

// MARK: - QR-Poll Pair Token Parser

public enum FabushiMcpQrPairParser: Sendable {
    /// Extracts a `pair_...` token from either a raw token string or a scanned QR URI
    /// (`fabushi://pair?pair_token=...`, `https://.../oauth/authorize/qr-poll?tx_id=...&pair_token=...`).
    public static func extractPairToken(from rawPayload: String) -> String? {
        let trimmed = rawPayload.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }

        if let components = URLComponents(string: trimmed),
           let items = components.queryItems
        {
            if let value = items.first(where: {
                $0.name == "pair_token" || $0.name == "token" || $0.name == "code"
            })?.value?.trimmingCharacters(in: .whitespacesAndNewlines),
               !value.isEmpty
            {
                return value
            }
        }

        if let range = trimmed.range(of: "pair_token=") {
            let tail = trimmed[range.upperBound...]
            let end = tail.firstIndex(of: "&") ?? tail.endIndex
            let token = String(tail[..<end]).removingPercentEncoding ?? String(tail[..<end])
            let clean = token.trimmingCharacters(in: .whitespacesAndNewlines)
            if !clean.isEmpty {
                return clean
            }
        }

        // Reject whitespace-containing junk when treated as raw token
        if trimmed.contains(where: \.isWhitespace) {
            return nil
        }
        return trimmed
    }
}

// MARK: - Exact `mcp_tool_audit` Redactor (`official_mcp.rs:L771-L788`)

/// Exact pure-Swift mirror of `official_mcp.rs:L771-L788` `summarize_tool_args`:
/// - Replaces string values of top-level JSON object keys `["code", "text", "content", "data", "password"]`
///   with `"<N chars>"` (exact Unicode scalar count).
/// - Applies `FabushiStudioSecretRedactor` defense-in-depth to remaining values.
/// - Truncates serialized JSON to 280 Unicode scalars (`first 277 + "..."`).
public enum FabushiMcpAuditRedactor: Sendable {
    public static let sensitiveTopLevelKeys: Set<String> = [
        "code",
        "text",
        "content",
        "data",
        "password",
    ]
    public static let maxSummaryChars: Int = 280

    public static func redactArgsSummary(_ rawJSONOrText: String) -> String {
        let trimmed = rawJSONOrText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "{}" }

        let secretRedactor = FabushiStudioSecretRedactor()

        if let data = trimmed.data(using: .utf8),
           let parsed = try? JSONDecoder().decode(FabushiJSONValue.self, from: data)
        {
            let sanitized = sanitizeTopLevelValue(parsed, secretRedactor: secretRedactor)
            let serialized = serializeCompactSorted(sanitized)
            return truncateToMaxChars(serialized)
        }

        let fallback = secretRedactor.redactText(trimmed).redactedText
        return truncateToMaxChars(fallback)
    }

    public static func redactRecord(_ record: FabushiMcpToolAuditRecordDTO) -> FabushiMcpToolAuditRecordDTO {
        FabushiMcpToolAuditRecordDTO(
            auditId: record.auditId,
            userId: record.userId,
            clientId: record.clientId,
            familyId: record.familyId,
            sessionId: record.sessionId,
            toolName: record.toolName,
            targetNodeId: record.targetNodeId,
            argsSummary: redactArgsSummary(record.argsSummary),
            ok: record.ok,
            errorCode: record.errorCode,
            durationMs: record.durationMs,
            createdAtMs: record.createdAtMs,
            source: record.source
        )
    }

    private static func sanitizeTopLevelValue(
        _ value: FabushiJSONValue,
        secretRedactor: FabushiStudioSecretRedactor
    ) -> FabushiJSONValue {
        switch value {
        case .object(let dict):
            var out: [String: FabushiJSONValue] = [:]
            out.reserveCapacity(dict.count)
            for (key, val) in dict {
                if sensitiveTopLevelKeys.contains(key),
                   case .string(let sensitiveStr) = val
                {
                    out[key] = .string("<\(sensitiveStr.unicodeScalars.count) chars>")
                } else {
                    out[key] = sanitizeNestedValue(val, secretRedactor: secretRedactor)
                }
            }
            return .object(out)
        default:
            return sanitizeNestedValue(value, secretRedactor: secretRedactor)
        }
    }

    private static func sanitizeNestedValue(
        _ value: FabushiJSONValue,
        secretRedactor: FabushiStudioSecretRedactor
    ) -> FabushiJSONValue {
        switch value {
        case .string(let str):
            return .string(secretRedactor.redactText(str).redactedText)
        case .array(let items):
            return .array(items.map { sanitizeNestedValue($0, secretRedactor: secretRedactor) })
        case .object(let dict):
            var out: [String: FabushiJSONValue] = [:]
            out.reserveCapacity(dict.count)
            for (k, v) in dict {
                out[k] = sanitizeNestedValue(v, secretRedactor: secretRedactor)
            }
            return .object(out)
        case .null, .bool, .int, .double:
            return value
        }
    }

    private static func serializeCompactSorted(_ value: FabushiJSONValue) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        if let encoded = try? encoder.encode(value) {
            return String(decoding: encoded, as: UTF8.self)
        }
        return value.toJSONString()
    }

    private static func truncateToMaxChars(_ text: String) -> String {
        let scalars = Array(text.unicodeScalars)
        guard scalars.count > maxSummaryChars else {
            return text
        }
        let prefixScalars = scalars.prefix(maxSummaryChars - 3)
        return String(String.UnicodeScalarView(prefixScalars)) + "..."
    }
}

// MARK: - OAuth 2.1 Grants, Servers, Tools, Audit & Kill-Switch DTOs (`mcp.*`, `auth.qrPairAccept`, `remote.sessionsDisconnectAll`)

public struct FabushiMcpOAuthGrantDTO: Codable, Sendable, Equatable, Identifiable {
    public let familyId: String
    public let clientId: String
    public let clientName: String
    public let scopes: [String]
    public let allowedNodeIds: [String]
    public let createdAtMs: Int64
    public let lastUsedMs: Int64?
    public let expiresAtMs: Int64

    public var id: String { familyId }

    public var allowsAllDevices: Bool {
        allowedNodeIds.isEmpty || allowedNodeIds.contains("*")
    }

    public enum CodingKeys: String, CodingKey {
        case familyId = "family_id"
        case clientId = "client_id"
        case clientName = "client_name"
        case scopes
        case allowedNodeIds = "allowed_node_ids"
        case createdAtMs = "created_at_ms"
        case lastUsedMs = "last_used_ms"
        case expiresAtMs = "expires_at_ms"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.familyId = try c.decode(String.self, forKey: .familyId)
        self.clientId = (try? c.decodeIfPresent(String.self, forKey: .clientId)) ?? ""
        self.clientName = (try? c.decodeIfPresent(String.self, forKey: .clientName)) ?? clientId
        self.scopes = (try? c.decodeIfPresent([String].self, forKey: .scopes)) ?? []
        self.allowedNodeIds = (try? c.decodeIfPresent([String].self, forKey: .allowedNodeIds)) ?? []
        self.createdAtMs = (try? c.decodeIfPresent(Int64.self, forKey: .createdAtMs)) ?? 0
        self.lastUsedMs = try? c.decodeIfPresent(Int64.self, forKey: .lastUsedMs)
        self.expiresAtMs = (try? c.decodeIfPresent(Int64.self, forKey: .expiresAtMs)) ?? 0
    }
}

public struct FabushiMcpOAuthGrantsResponse: Codable, Sendable, Equatable {
    public let grants: [FabushiMcpOAuthGrantDTO]
}

public struct FabushiMcpRevokeGrantRequest: Encodable, Sendable, Equatable {
    public let familyId: String

    public enum CodingKeys: String, CodingKey {
        case familyId = "family_id"
    }
}

public struct FabushiMcpRevokeGrantResponse: Codable, Sendable, Equatable {
    public let revoked: Bool
    public let familyId: String

    public enum CodingKeys: String, CodingKey {
        case revoked
        case familyId = "family_id"
    }
}

public enum FabushiMcpTransportKind: String, CaseIterable, Codable, Sendable, Equatable, Identifiable {
    case streamableHttp = "streamable_http"
    case sse = "sse"
    case stdio = "stdio"

    public var id: String { rawValue }

    public var displayName: String {
        switch self {
        case .streamableHttp: return "Streamable HTTP (HTTPS)"
        case .sse: return "Server-Sent Events (HTTPS)"
        case .stdio: return "Local stdio (Blocked on iOS)"
        }
    }

    public var isAllowedOnIos: Bool {
        self != .stdio
    }
}

public struct FabushiMcpRegisterServerRequest: Encodable, Sendable, Equatable {
    public let name: String
    public let transport: String
    public let url: String?
    public let command: String?

    public init(name: String, transport: String, url: String? = nil, command: String? = nil) {
        self.name = name
        self.transport = transport
        self.url = url
        self.command = command
    }
}

public struct FabushiMcpRemoteServerDTO: Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let transport: String
    public let url: String?
    public let status: String

    public init(from value: FabushiJSONValue, fallbackIndex: Int) {
        let serverId = value["id"]?.stringValue
            ?? value["server_id"]?.stringValue
            ?? value["name"]?.stringValue
            ?? "mcp_server_\(fallbackIndex)"
        self.id = serverId
        self.name = value["name"]?.stringValue ?? serverId
        self.transport = value["transport"]?.stringValue ?? "streamable_http"
        self.url = value["url"]?.stringValue ?? value["endpoint"]?.stringValue
        self.status = value["status"]?.stringValue ?? "registered"
    }
}

public struct FabushiMcpLocalToolDTO: Codable, Sendable, Equatable, Identifiable {
    public let name: String
    public let description: String
    public let inputSchemaJson: String
    public let requiresApproval: Bool
    public let riskLevel: String
    public let serverId: String?

    public var id: String { "\(serverId ?? "local"):\(name)" }

    public enum CodingKeys: String, CodingKey {
        case name
        case description
        case inputSchemaJson = "input_schema_json"
        case requiresApproval = "requires_approval"
        case riskLevel = "risk_level"
        case serverId = "server_id"
    }
}

public struct FabushiMcpCallToolRequest: Encodable, Sendable, Equatable {
    public let serverId: String?
    public let toolName: String
    public let arguments: FabushiJSONValue

    public enum CodingKeys: String, CodingKey {
        case serverId = "server_id"
        case toolName = "tool_name"
        case arguments
    }
}

public struct FabushiMcpApprovePermissionRequest: Encodable, Sendable, Equatable {
    public let requestId: String
    public let approved: Bool

    public enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case approved
    }
}

public struct FabushiMcpQrPairAcceptRequest: Encodable, Sendable, Equatable {
    public let pairToken: String

    public enum CodingKeys: String, CodingKey {
        case pairToken = "pair_token"
    }
}

public struct FabushiMcpQrPairAcceptResponse: Codable, Sendable, Equatable {
    public let accepted: Bool
}

public enum FabushiMcpAuditRecordSource: String, Codable, Sendable, Equatable {
    case serverMcpAudit = "server_mcp_audit"
    case localApprovalLedger = "local_approval_ledger"
}

public enum FabushiMcpAuditStatusFilter: String, CaseIterable, Sendable, Equatable, Identifiable {
    case all
    case allowed
    case failed

    public var id: String { rawValue }

    public var localizationKey: String {
        switch self {
        case .all: return "m4.mcp.filterAll"
        case .allowed: return "m4.mcp.filterOk"
        case .failed: return "m4.mcp.filterFailed"
        }
    }
}

public struct FabushiMcpToolAuditRecordDTO: Codable, Sendable, Equatable, Identifiable {
    public let auditId: String
    public let userId: UInt64
    public let clientId: String
    public let familyId: String
    public let sessionId: String
    public let toolName: String
    public let targetNodeId: String?
    public let argsSummary: String
    public let ok: Bool
    public let errorCode: String?
    public let durationMs: Int64
    public let createdAtMs: Int64
    public let source: FabushiMcpAuditRecordSource

    public var id: String { auditId }

    public var inferredScope: String {
        FabushiMcpOfficialCatalog.requiredScope(for: toolName) ?? "local:approval"
    }

    public enum CodingKeys: String, CodingKey {
        case auditId = "audit_id"
        case userId = "user_id"
        case clientId = "client_id"
        case familyId = "family_id"
        case sessionId = "session_id"
        case toolName = "tool_name"
        case targetNodeId = "target_node_id"
        case argsSummary = "args_summary"
        case ok
        case errorCode = "error_code"
        case durationMs = "duration_ms"
        case createdAtMs = "created_at_ms"
        case source
    }

    public init(
        auditId: String,
        userId: UInt64 = 0,
        clientId: String = "",
        familyId: String = "",
        sessionId: String = "",
        toolName: String,
        targetNodeId: String? = nil,
        argsSummary: String,
        ok: Bool,
        errorCode: String? = nil,
        durationMs: Int64 = 0,
        createdAtMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000),
        source: FabushiMcpAuditRecordSource = .serverMcpAudit
    ) {
        self.auditId = auditId
        self.userId = userId
        self.clientId = clientId
        self.familyId = familyId
        self.sessionId = sessionId
        self.toolName = toolName
        self.targetNodeId = targetNodeId
        self.argsSummary = argsSummary
        self.ok = ok
        self.errorCode = errorCode
        self.durationMs = durationMs
        self.createdAtMs = createdAtMs
        self.source = source
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.auditId = try c.decode(String.self, forKey: .auditId)
        self.userId = (try? c.decodeIfPresent(UInt64.self, forKey: .userId)) ?? 0
        self.clientId = (try? c.decodeIfPresent(String.self, forKey: .clientId)) ?? ""
        self.familyId = (try? c.decodeIfPresent(String.self, forKey: .familyId)) ?? ""
        self.sessionId = (try? c.decodeIfPresent(String.self, forKey: .sessionId)) ?? ""
        self.toolName = (try? c.decodeIfPresent(String.self, forKey: .toolName)) ?? "unknown_tool"
        self.targetNodeId = try? c.decodeIfPresent(String.self, forKey: .targetNodeId)
        self.argsSummary = (try? c.decodeIfPresent(String.self, forKey: .argsSummary)) ?? "{}"
        self.ok = (try? c.decodeIfPresent(Bool.self, forKey: .ok)) ?? true
        self.errorCode = try? c.decodeIfPresent(String.self, forKey: .errorCode)
        self.durationMs = (try? c.decodeIfPresent(Int64.self, forKey: .durationMs)) ?? 0
        self.createdAtMs = (try? c.decodeIfPresent(Int64.self, forKey: .createdAtMs)) ?? 0
        self.source = (try? c.decodeIfPresent(FabushiMcpAuditRecordSource.self, forKey: .source)) ?? .serverMcpAudit
    }
}
