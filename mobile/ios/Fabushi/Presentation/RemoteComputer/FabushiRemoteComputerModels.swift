import Foundation
import UIKit

// MARK: - Remote Computer Enums (Exact Rust snake_case Serde Parity)

public enum FabushiComputerNodeKind: String, Codable, Sendable, Equatable, CaseIterable {
    case localDevice = "local_device"
    case cloudServer = "cloud_server"
    case cloudSandbox = "cloud_sandbox"
}

public enum FabushiComputerRunPolicy: String, Codable, Sendable, Equatable, CaseIterable, Identifiable {
    case askEveryTime = "ask_every_time"
    case allowReadOnly = "allow_read_only"
    case alwaysAllowAuthorized = "always_allow_authorized"
    case disabled = "disabled"

    public var id: String { rawValue }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .askEveryTime:
            return FabushiPlatformStrings.text(.remotePolicyAskEveryTime, locale: locale)
        case .allowReadOnly:
            return FabushiPlatformStrings.text(.remotePolicyAllowReadOnly, locale: locale)
        case .alwaysAllowAuthorized:
            return FabushiPlatformStrings.text(.remotePolicyAlwaysAllow, locale: locale)
        case .disabled:
            return FabushiPlatformStrings.text(.remotePolicyDisabled, locale: locale)
        }
    }
}

public enum FabushiCloudAuthMethod: String, Codable, Sendable, Equatable, CaseIterable, Identifiable {
    case sshPassword = "ssh_password"
    case sshPrivateKey = "ssh_private_key"
    case agentHttpToken = "agent_http_token"

    public var id: String { rawValue }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .sshPassword:
            return FabushiPlatformStrings.text(locale, en: "SSH Password", zhHans: "SSH 密码")
        case .sshPrivateKey:
            return FabushiPlatformStrings.text(locale, en: "SSH Private Key", zhHans: "SSH 私钥")
        case .agentHttpToken:
            return FabushiPlatformStrings.text(locale, en: "Agent HTTP Token", zhHans: "Agent HTTP 令牌")
        }
    }
}

public enum FabushiAuthorizedControllerKind: String, Codable, Sendable, Equatable {
    case device = "device"
    case bot = "bot"
    case accountAllBots = "account_all_bots"
}

public enum FabushiRemoteControllerKind: String, Codable, Sendable, Equatable {
    case ownDevice = "own_device"
    case assistGuest = "assist_guest"
    case aiMcp = "ai_mcp"
    case cli = "cli"

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .ownDevice:
            return FabushiPlatformStrings.text(locale, en: "Own Device", zhHans: "本账号设备")
        case .assistGuest:
            return FabushiPlatformStrings.text(locale, en: "Assist Guest", zhHans: "远程协助设备")
        case .aiMcp:
            return FabushiPlatformStrings.text(locale, en: "AI Agent (MCP)", zhHans: "AI Agent (MCP)")
        case .cli:
            return FabushiPlatformStrings.text(locale, en: "Local CLI", zhHans: "本地 CLI")
        }
    }
}

public enum FabushiRemoteControlMode: String, Codable, Sendable, Equatable, CaseIterable, Identifiable {
    case desktop = "desktop"
    case terminal = "terminal"
    case fileTransfer = "file_transfer"
    case aiControl = "ai_control"

    public var id: String { rawValue }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .desktop:
            return FabushiPlatformStrings.text(locale, en: "Remote Desktop", zhHans: "远程桌面")
        case .terminal:
            return FabushiPlatformStrings.text(locale, en: "Remote Terminal", zhHans: "远程终端")
        case .fileTransfer:
            return FabushiPlatformStrings.text(locale, en: "File Transfer", zhHans: "文件传输")
        case .aiControl:
            return FabushiPlatformStrings.text(locale, en: "AI Control", zhHans: "AI 操控")
        }
    }
}

public enum FabushiAssistCodeMode: String, Codable, Sendable, Equatable {
    case temporary = "temporary"
    case custom = "custom"
}

public enum FabushiTailnetRelayMode: String, Codable, Sendable, Equatable {
    case directUdp = "direct_udp"
    case derpRelay = "derp_relay"
    case httpFallback = "http_fallback"
    case offline = "offline"

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .directUdp:
            return FabushiPlatformStrings.text(.remoteTailnetDirectP2P, locale: locale)
        case .derpRelay:
            return FabushiPlatformStrings.text(.remoteTailnetDerpRelay, locale: locale)
        case .httpFallback:
            return FabushiPlatformStrings.text(.remoteTailnetHttpFallback, locale: locale)
        case .offline:
            return FabushiPlatformStrings.text(.remoteTailnetOffline, locale: locale)
        }
    }
}

// MARK: - Power Action & RemoteComputerOp (`#[serde(tag = "op", rename_all = "snake_case")]`)

public enum FabushiPowerActionKind: Sendable, Equatable, Codable {
    case lock
    case sleep
    case reboot
    case shutdown
    case scheduleShutdown(delayMinutes: UInt32)
    case wakeOnLan(macAddress: String, broadcastIp: String?)

    private enum CodingKeys: String, CodingKey {
        case kind
        case delayMinutes = "delay_minutes"
        case macAddress = "mac_address"
        case broadcastIp = "broadcast_ip"
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(String.self, forKey: .kind)
        switch kind {
        case "lock":
            self = .lock
        case "sleep":
            self = .sleep
        case "reboot":
            self = .reboot
        case "shutdown":
            self = .shutdown
        case "schedule_shutdown":
            let delay = try container.decodeIfPresent(UInt32.self, forKey: .delayMinutes) ?? 5
            self = .scheduleShutdown(delayMinutes: delay)
        case "wake_on_lan":
            let mac = try container.decode(String.self, forKey: .macAddress)
            let broadcast = try container.decodeIfPresent(String.self, forKey: .broadcastIp)
            self = .wakeOnLan(macAddress: mac, broadcastIp: broadcast)
        default:
            self = .lock
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .lock:
            try container.encode("lock", forKey: .kind)
        case .sleep:
            try container.encode("sleep", forKey: .kind)
        case .reboot:
            try container.encode("reboot", forKey: .kind)
        case .shutdown:
            try container.encode("shutdown", forKey: .kind)
        case .scheduleShutdown(let delayMinutes):
            try container.encode("schedule_shutdown", forKey: .kind)
            try container.encode(delayMinutes, forKey: .delayMinutes)
        case .wakeOnLan(let macAddress, let broadcastIp):
            try container.encode("wake_on_lan", forKey: .kind)
            try container.encode(macAddress, forKey: .macAddress)
            try container.encodeIfPresent(broadcastIp, forKey: .broadcastIp)
        }
    }
}

public enum FabushiRemoteComputerOp: Sendable, Equatable, Codable {
    case shellExec(command: String, cwd: String? = nil, timeoutMs: UInt64? = nil)
    case readFile(path: String, maxBytes: Int? = nil)
    case writeFile(path: String, content: String, createDirs: Bool = false)
    case listDir(path: String)
    case captureScreen
    case mouseAction(
        action: String,
        x: Int32? = nil,
        y: Int32? = nil,
        dx: Int32? = nil,
        dy: Int32? = nil,
        button: String? = nil,
        clicks: UInt8? = nil,
        scrollX: Int32? = nil,
        scrollY: Int32? = nil,
        relative: Bool = false
    )
    case keyboardAction(
        action: String,
        text: String? = nil,
        key: String? = nil,
        modifiers: [String] = []
    )
    case clipboardGet
    case clipboardSet(text: String? = nil, imageB64: String? = nil, mimeType: String? = nil)
    case powerAction(action: FabushiPowerActionKind)
    case displayList
    case displaySwitch(displayIndex: UInt32)
    case privacyScreenToggle(enabled: Bool, message: String? = nil)
    case terminalOp(
        terminalOp: String,
        sessionName: String? = nil,
        shell: String? = nil,
        input: String? = nil,
        command: String? = nil,
        cols: UInt16? = nil,
        rows: UInt16? = nil,
        timeoutMs: UInt64? = nil,
        newName: String? = nil
    )
    case testConnection

    private enum CodingKeys: String, CodingKey {
        case op
        case command
        case cwd
        case timeoutMs = "timeout_ms"
        case path
        case maxBytes = "max_bytes"
        case content
        case createDirs = "create_dirs"
        case action
        case x, y, dx, dy
        case button, clicks
        case scrollX = "scroll_x"
        case scrollY = "scroll_y"
        case relative
        case text, key, modifiers
        case imageB64 = "image_b64"
        case mimeType = "mime_type"
        case displayIndex = "display_index"
        case enabled, message
        case terminalOp = "terminal_op"
        case sessionName = "session_name"
        case shell, input, cols, rows
        case newName = "new_name"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let op = try c.decodeIfPresent(String.self, forKey: .op) ?? "test_connection"
        switch op {
        case "shell_exec":
            self = .shellExec(
                command: try c.decodeIfPresent(String.self, forKey: .command) ?? "",
                cwd: try c.decodeIfPresent(String.self, forKey: .cwd),
                timeoutMs: try c.decodeIfPresent(UInt64.self, forKey: .timeoutMs)
            )
        case "read_file":
            self = .readFile(
                path: try c.decodeIfPresent(String.self, forKey: .path) ?? "",
                maxBytes: try c.decodeIfPresent(Int.self, forKey: .maxBytes)
            )
        case "write_file":
            self = .writeFile(
                path: try c.decodeIfPresent(String.self, forKey: .path) ?? "",
                content: try c.decodeIfPresent(String.self, forKey: .content) ?? "",
                createDirs: try c.decodeIfPresent(Bool.self, forKey: .createDirs) ?? false
            )
        case "list_dir":
            self = .listDir(path: try c.decodeIfPresent(String.self, forKey: .path) ?? ".")
        case "capture_screen":
            self = .captureScreen
        case "mouse_action":
            self = .mouseAction(
                action: try c.decodeIfPresent(String.self, forKey: .action) ?? "move",
                x: try c.decodeIfPresent(Int32.self, forKey: .x),
                y: try c.decodeIfPresent(Int32.self, forKey: .y),
                dx: try c.decodeIfPresent(Int32.self, forKey: .dx),
                dy: try c.decodeIfPresent(Int32.self, forKey: .dy),
                button: try c.decodeIfPresent(String.self, forKey: .button),
                clicks: try c.decodeIfPresent(UInt8.self, forKey: .clicks),
                scrollX: try c.decodeIfPresent(Int32.self, forKey: .scrollX),
                scrollY: try c.decodeIfPresent(Int32.self, forKey: .scrollY),
                relative: try c.decodeIfPresent(Bool.self, forKey: .relative) ?? false
            )
        case "keyboard_action":
            self = .keyboardAction(
                action: try c.decodeIfPresent(String.self, forKey: .action) ?? "press",
                text: try c.decodeIfPresent(String.self, forKey: .text),
                key: try c.decodeIfPresent(String.self, forKey: .key),
                modifiers: try c.decodeIfPresent([String].self, forKey: .modifiers) ?? []
            )
        case "clipboard_get":
            self = .clipboardGet
        case "clipboard_set":
            self = .clipboardSet(
                text: try c.decodeIfPresent(String.self, forKey: .text),
                imageB64: try c.decodeIfPresent(String.self, forKey: .imageB64),
                mimeType: try c.decodeIfPresent(String.self, forKey: .mimeType)
            )
        case "power_action":
            let kind = try c.decodeIfPresent(FabushiPowerActionKind.self, forKey: .action) ?? .lock
            self = .powerAction(action: kind)
        case "display_list":
            self = .displayList
        case "display_switch":
            self = .displaySwitch(displayIndex: try c.decodeIfPresent(UInt32.self, forKey: .displayIndex) ?? 0)
        case "privacy_screen_toggle":
            self = .privacyScreenToggle(
                enabled: try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? false,
                message: try c.decodeIfPresent(String.self, forKey: .message)
            )
        case "terminal_op":
            self = .terminalOp(
                terminalOp: try c.decodeIfPresent(String.self, forKey: .terminalOp) ?? "ls",
                sessionName: try c.decodeIfPresent(String.self, forKey: .sessionName),
                shell: try c.decodeIfPresent(String.self, forKey: .shell),
                input: try c.decodeIfPresent(String.self, forKey: .input),
                command: try c.decodeIfPresent(String.self, forKey: .command),
                cols: try c.decodeIfPresent(UInt16.self, forKey: .cols),
                rows: try c.decodeIfPresent(UInt16.self, forKey: .rows),
                timeoutMs: try c.decodeIfPresent(UInt64.self, forKey: .timeoutMs),
                newName: try c.decodeIfPresent(String.self, forKey: .newName)
            )
        default:
            self = .testConnection
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .shellExec(let command, let cwd, let timeoutMs):
            try c.encode("shell_exec", forKey: .op)
            try c.encode(command, forKey: .command)
            try c.encodeIfPresent(cwd, forKey: .cwd)
            try c.encodeIfPresent(timeoutMs, forKey: .timeoutMs)
        case .readFile(let path, let maxBytes):
            try c.encode("read_file", forKey: .op)
            try c.encode(path, forKey: .path)
            try c.encodeIfPresent(maxBytes, forKey: .maxBytes)
        case .writeFile(let path, let content, let createDirs):
            try c.encode("write_file", forKey: .op)
            try c.encode(path, forKey: .path)
            try c.encode(content, forKey: .content)
            try c.encode(createDirs, forKey: .createDirs)
        case .listDir(let path):
            try c.encode("list_dir", forKey: .op)
            try c.encode(path, forKey: .path)
        case .captureScreen:
            try c.encode("capture_screen", forKey: .op)
        case .mouseAction(let action, let x, let y, let dx, let dy, let button, let clicks, let scrollX, let scrollY, let relative):
            try c.encode("mouse_action", forKey: .op)
            try c.encode(action, forKey: .action)
            try c.encodeIfPresent(x, forKey: .x)
            try c.encodeIfPresent(y, forKey: .y)
            try c.encodeIfPresent(dx, forKey: .dx)
            try c.encodeIfPresent(dy, forKey: .dy)
            try c.encodeIfPresent(button, forKey: .button)
            try c.encodeIfPresent(clicks, forKey: .clicks)
            try c.encodeIfPresent(scrollX, forKey: .scrollX)
            try c.encodeIfPresent(scrollY, forKey: .scrollY)
            try c.encode(relative, forKey: .relative)
        case .keyboardAction(let action, let text, let key, let modifiers):
            try c.encode("keyboard_action", forKey: .op)
            try c.encode(action, forKey: .action)
            try c.encodeIfPresent(text, forKey: .text)
            try c.encodeIfPresent(key, forKey: .key)
            try c.encode(modifiers, forKey: .modifiers)
        case .clipboardGet:
            try c.encode("clipboard_get", forKey: .op)
        case .clipboardSet(let text, let imageB64, let mimeType):
            try c.encode("clipboard_set", forKey: .op)
            try c.encodeIfPresent(text, forKey: .text)
            try c.encodeIfPresent(imageB64, forKey: .imageB64)
            try c.encodeIfPresent(mimeType, forKey: .mimeType)
        case .powerAction(let action):
            try c.encode("power_action", forKey: .op)
            try c.encode(action, forKey: .action)
        case .displayList:
            try c.encode("display_list", forKey: .op)
        case .displaySwitch(let displayIndex):
            try c.encode("display_switch", forKey: .op)
            try c.encode(displayIndex, forKey: .displayIndex)
        case .privacyScreenToggle(let enabled, let message):
            try c.encode("privacy_screen_toggle", forKey: .op)
            try c.encode(enabled, forKey: .enabled)
            try c.encodeIfPresent(message, forKey: .message)
        case .terminalOp(let terminalOp, let sessionName, let shell, let input, let command, let cols, let rows, let timeoutMs, let newName):
            try c.encode("terminal_op", forKey: .op)
            try c.encode(terminalOp, forKey: .terminalOp)
            try c.encodeIfPresent(sessionName, forKey: .sessionName)
            try c.encodeIfPresent(shell, forKey: .shell)
            try c.encodeIfPresent(input, forKey: .input)
            try c.encodeIfPresent(command, forKey: .command)
            try c.encodeIfPresent(cols, forKey: .cols)
            try c.encodeIfPresent(rows, forKey: .rows)
            try c.encodeIfPresent(timeoutMs, forKey: .timeoutMs)
            try c.encodeIfPresent(newName, forKey: .newName)
        case .testConnection:
            try c.encode("test_connection", forKey: .op)
        }
    }

    public var summaryLabel: String {
        switch self {
        case .shellExec(let cmd, _, _): return "shell_exec: \(cmd)"
        case .readFile(let path, _): return "read_file: \(path)"
        case .writeFile(let path, _, _): return "write_file: \(path)"
        case .listDir(let path): return "list_dir: \(path)"
        case .captureScreen: return "capture_screen"
        case .mouseAction(let action, _, _, _, _, _, _, _, _, _): return "mouse_action: \(action)"
        case .keyboardAction(let action, _, let key, _): return "keyboard_action: \(action) \(key ?? "")"
        case .clipboardGet: return "clipboard_get"
        case .clipboardSet: return "clipboard_set"
        case .powerAction: return "power_action"
        case .displayList: return "display_list"
        case .displaySwitch(let idx): return "display_switch: #\(idx)"
        case .privacyScreenToggle(let enabled, _): return "privacy_screen: \(enabled)"
        case .terminalOp(let op, let session, _, _, _, _, _, _, _):
            return "terminal_op: \(op) (\(session ?? "default"))"
        case .testConnection: return "test_connection"
        }
    }
}

// MARK: - Core Remote Computer DTOs

public struct FabushiAuthorizedControllerEntry: Codable, Sendable, Equatable, Identifiable {
    public var id: String { controllerId }
    public let controllerId: String
    public let kind: FabushiAuthorizedControllerKind
    public let displayName: String
    public let addedAtMs: Int64

    public init(
        controllerId: String,
        kind: FabushiAuthorizedControllerKind,
        displayName: String,
        addedAtMs: Int64 = 0
    ) {
        self.controllerId = controllerId
        self.kind = kind
        self.displayName = displayName
        self.addedAtMs = addedAtMs
    }

    enum CodingKeys: String, CodingKey {
        case controllerId = "controller_id"
        case kind
        case displayName = "display_name"
        case addedAtMs = "added_at_ms"
    }
}

public struct FabushiComputerNodeDto: Codable, Sendable, Equatable, Identifiable {
    public var id: String { nodeId }
    public let nodeId: String
    public let ownerUserId: UInt64
    public let kind: FabushiComputerNodeKind
    public let displayName: String
    public let ipAddress: String
    public let host: String?
    public let sshPort: UInt16?
    public let sshUsername: String?
    public let authMethod: FabushiCloudAuthMethod?
    public let agentEndpointUrl: String?
    public let workdir: String?
    public let hasSavedCredential: Bool
    public let osName: String
    public let resolution: String
    public let online: Bool
    public let isCurrentDevice: Bool
    public let deviceId: String?
    public let tierBadge: String
    public let runPolicy: FabushiComputerRunPolicy
    public let authorizedControllers: [FabushiAuthorizedControllerEntry]
    public let boundBotIds: [UInt64]
    public let lastSeenMs: Int64
    public let latestFrameDataUrl: String?

    public init(
        nodeId: String,
        ownerUserId: UInt64 = 0,
        kind: FabushiComputerNodeKind = .localDevice,
        displayName: String,
        ipAddress: String = "",
        host: String? = nil,
        sshPort: UInt16? = nil,
        sshUsername: String? = nil,
        authMethod: FabushiCloudAuthMethod? = nil,
        agentEndpointUrl: String? = nil,
        workdir: String? = nil,
        hasSavedCredential: Bool = false,
        osName: String = "",
        resolution: String = "",
        online: Bool = false,
        isCurrentDevice: Bool = false,
        deviceId: String? = nil,
        tierBadge: String = "",
        runPolicy: FabushiComputerRunPolicy = .askEveryTime,
        authorizedControllers: [FabushiAuthorizedControllerEntry] = [],
        boundBotIds: [UInt64] = [],
        lastSeenMs: Int64 = 0,
        latestFrameDataUrl: String? = nil
    ) {
        self.nodeId = nodeId
        self.ownerUserId = ownerUserId
        self.kind = kind
        self.displayName = displayName
        self.ipAddress = ipAddress
        self.host = host
        self.sshPort = sshPort
        self.sshUsername = sshUsername
        self.authMethod = authMethod
        self.agentEndpointUrl = agentEndpointUrl
        self.workdir = workdir
        self.hasSavedCredential = hasSavedCredential
        self.osName = osName
        self.resolution = resolution
        self.online = online
        self.isCurrentDevice = isCurrentDevice
        self.deviceId = deviceId
        self.tierBadge = tierBadge
        self.runPolicy = runPolicy
        self.authorizedControllers = authorizedControllers
        self.boundBotIds = boundBotIds
        self.lastSeenMs = lastSeenMs
        self.latestFrameDataUrl = latestFrameDataUrl
    }

    enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case ownerUserId = "owner_user_id"
        case kind
        case displayName = "display_name"
        case ipAddress = "ip_address"
        case host
        case sshPort = "ssh_port"
        case sshUsername = "ssh_username"
        case authMethod = "auth_method"
        case agentEndpointUrl = "agent_endpoint_url"
        case workdir
        case hasSavedCredential = "has_saved_credential"
        case osName = "os_name"
        case resolution
        case online
        case isCurrentDevice = "is_current_device"
        case deviceId = "device_id"
        case tierBadge = "tier_badge"
        case runPolicy = "run_policy"
        case authorizedControllers = "authorized_controllers"
        case boundBotIds = "bound_bot_ids"
        case lastSeenMs = "last_seen_ms"
        case latestFrameDataUrl = "latest_frame_data_url"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nodeId = try c.decodeIfPresent(String.self, forKey: .nodeId) ?? ""
        ownerUserId = try c.decodeIfPresent(UInt64.self, forKey: .ownerUserId) ?? 0
        kind = try c.decodeIfPresent(FabushiComputerNodeKind.self, forKey: .kind) ?? .localDevice
        displayName = try c.decodeIfPresent(String.self, forKey: .displayName) ?? ""
        ipAddress = try c.decodeIfPresent(String.self, forKey: .ipAddress) ?? ""
        host = try c.decodeIfPresent(String.self, forKey: .host)
        sshPort = try c.decodeIfPresent(UInt16.self, forKey: .sshPort)
        sshUsername = try c.decodeIfPresent(String.self, forKey: .sshUsername)
        authMethod = try c.decodeIfPresent(FabushiCloudAuthMethod.self, forKey: .authMethod)
        agentEndpointUrl = try c.decodeIfPresent(String.self, forKey: .agentEndpointUrl)
        workdir = try c.decodeIfPresent(String.self, forKey: .workdir)
        hasSavedCredential = try c.decodeIfPresent(Bool.self, forKey: .hasSavedCredential) ?? false
        osName = try c.decodeIfPresent(String.self, forKey: .osName) ?? ""
        resolution = try c.decodeIfPresent(String.self, forKey: .resolution) ?? ""
        online = try c.decodeIfPresent(Bool.self, forKey: .online) ?? false
        isCurrentDevice = try c.decodeIfPresent(Bool.self, forKey: .isCurrentDevice) ?? false
        deviceId = try c.decodeIfPresent(String.self, forKey: .deviceId)
        tierBadge = try c.decodeIfPresent(String.self, forKey: .tierBadge) ?? ""
        runPolicy = try c.decodeIfPresent(FabushiComputerRunPolicy.self, forKey: .runPolicy) ?? .askEveryTime
        authorizedControllers = try c.decodeIfPresent([FabushiAuthorizedControllerEntry].self, forKey: .authorizedControllers) ?? []
        boundBotIds = try c.decodeIfPresent([UInt64].self, forKey: .boundBotIds) ?? []
        lastSeenMs = try c.decodeIfPresent(Int64.self, forKey: .lastSeenMs) ?? 0
        latestFrameDataUrl = try c.decodeIfPresent(String.self, forKey: .latestFrameDataUrl)
    }

    public var isCloudNode: Bool {
        kind == .cloudServer || kind == .cloudSandbox
    }

    public var osSymbolName: String {
        let lower = osName.lowercased()
        if isCloudNode {
            return "cloud.fill"
        }
        if lower.contains("mac") || lower.contains("darwin") {
            return "laptopcomputer"
        }
        if lower.contains("win") {
            return "pc"
        }
        if lower.contains("linux") || lower.contains("ubuntu") || lower.contains("debian") {
            return "server.rack"
        }
        return "desktopcomputer"
    }
}

public struct FabushiPendingComputerCommandDto: Codable, Sendable, Equatable, Identifiable {
    public var id: String { commandId }
    public let commandId: String
    public let nodeId: String
    public let targetDeviceId: String
    public let callerDeviceId: String?
    public let callerBotId: UInt64?
    public let callerLabel: String
    public let requiresUserApproval: Bool
    public let approved: Bool
    public let operation: FabushiRemoteComputerOp
    public let createdAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case commandId = "command_id"
        case nodeId = "node_id"
        case targetDeviceId = "target_device_id"
        case callerDeviceId = "caller_device_id"
        case callerBotId = "caller_bot_id"
        case callerLabel = "caller_label"
        case requiresUserApproval = "requires_user_approval"
        case approved
        case operation
        case createdAtMs = "created_at_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        commandId = try c.decodeIfPresent(String.self, forKey: .commandId) ?? ""
        nodeId = try c.decodeIfPresent(String.self, forKey: .nodeId) ?? ""
        targetDeviceId = try c.decodeIfPresent(String.self, forKey: .targetDeviceId) ?? ""
        callerDeviceId = try c.decodeIfPresent(String.self, forKey: .callerDeviceId)
        callerBotId = try c.decodeIfPresent(UInt64.self, forKey: .callerBotId)
        callerLabel = try c.decodeIfPresent(String.self, forKey: .callerLabel) ?? ""
        requiresUserApproval = try c.decodeIfPresent(Bool.self, forKey: .requiresUserApproval) ?? true
        approved = try c.decodeIfPresent(Bool.self, forKey: .approved) ?? false
        operation = try c.decodeIfPresent(FabushiRemoteComputerOp.self, forKey: .operation) ?? .testConnection
        createdAtMs = try c.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
    }
}

public struct FabushiComputerListResponse: Codable, Sendable, Equatable {
    public let computers: [FabushiComputerNodeDto]
    public let pendingApprovals: [FabushiPendingComputerCommandDto]

    enum CodingKeys: String, CodingKey {
        case computers
        case pendingApprovals = "pending_approvals"
    }

    public init(
        computers: [FabushiComputerNodeDto] = [],
        pendingApprovals: [FabushiPendingComputerCommandDto] = []
    ) {
        self.computers = computers
        self.pendingApprovals = pendingApprovals
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        computers = try c.decodeIfPresent([FabushiComputerNodeDto].self, forKey: .computers) ?? []
        pendingApprovals = try c.decodeIfPresent([FabushiPendingComputerCommandDto].self, forKey: .pendingApprovals) ?? []
    }
}

public struct FabushiUpsertCloudComputerRequest: Codable, Sendable, Equatable {
    public let nodeId: String?
    public let displayName: String
    public let host: String
    public let port: UInt16
    public let username: String
    public let authMethod: FabushiCloudAuthMethod
    public let secret: String?
    public let agentEndpointUrl: String?
    public let workdir: String?
    public let bindBotId: UInt64?
    public let verifyConnection: Bool

    public init(
        nodeId: String? = nil,
        displayName: String,
        host: String,
        port: UInt16 = 22,
        username: String,
        authMethod: FabushiCloudAuthMethod = .sshPassword,
        secret: String? = nil,
        agentEndpointUrl: String? = nil,
        workdir: String? = nil,
        bindBotId: UInt64? = nil,
        verifyConnection: Bool = true
    ) {
        self.nodeId = nodeId
        self.displayName = displayName
        self.host = host
        self.port = port
        self.username = username
        self.authMethod = authMethod
        self.secret = secret
        self.agentEndpointUrl = agentEndpointUrl
        self.workdir = workdir
        self.bindBotId = bindBotId
        self.verifyConnection = verifyConnection
    }

    enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case displayName = "display_name"
        case host
        case port
        case username
        case authMethod = "auth_method"
        case secret
        case agentEndpointUrl = "agent_endpoint_url"
        case workdir
        case bindBotId = "bind_bot_id"
        case verifyConnection = "verify_connection"
    }
}

public struct FabushiUpdateComputerPolicyRequest: Codable, Sendable, Equatable {
    public let runPolicy: FabushiComputerRunPolicy?
    public let addController: FabushiAuthorizedControllerEntry?
    public let removeControllerId: String?
    public let displayName: String?

    public init(
        runPolicy: FabushiComputerRunPolicy? = nil,
        addController: FabushiAuthorizedControllerEntry? = nil,
        removeControllerId: String? = nil,
        displayName: String? = nil
    ) {
        self.runPolicy = runPolicy
        self.addController = addController
        self.removeControllerId = removeControllerId
        self.displayName = displayName
    }

    enum CodingKeys: String, CodingKey {
        case runPolicy = "run_policy"
        case addController = "add_controller"
        case removeControllerId = "remove_controller_id"
        case displayName = "display_name"
    }
}

public struct FabushiRemoteComputerCommandRequest: Codable, Sendable, Equatable {
    public let nodeId: String
    public let callerDeviceId: String?
    public let callerBotId: UInt64?
    public let callerLabel: String
    public let operation: FabushiRemoteComputerOp

    public init(
        nodeId: String,
        callerDeviceId: String? = nil,
        callerBotId: UInt64? = nil,
        callerLabel: String = "Fabushi iOS",
        operation: FabushiRemoteComputerOp
    ) {
        self.nodeId = nodeId
        self.callerDeviceId = callerDeviceId
        self.callerBotId = callerBotId
        self.callerLabel = callerLabel
        self.operation = operation
    }

    enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case callerDeviceId = "caller_device_id"
        case callerBotId = "caller_bot_id"
        case callerLabel = "caller_label"
        case operation
    }
}

public struct FabushiRemoteComputerCommandResult: Codable, Sendable, Equatable {
    public let commandId: String
    public let nodeId: String
    public let ok: Bool
    public let exitCode: Int32
    public let stdout: String
    public let stderr: String
    public let entries: [String]
    public let frameDataUrl: String?
    public let executedBy: String
    public let durationMs: UInt64

    enum CodingKeys: String, CodingKey {
        case commandId = "command_id"
        case nodeId = "node_id"
        case ok
        case exitCode = "exit_code"
        case stdout
        case stderr
        case entries
        case frameDataUrl = "frame_data_url"
        case executedBy = "executed_by"
        case durationMs = "duration_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        commandId = try c.decodeIfPresent(String.self, forKey: .commandId) ?? ""
        nodeId = try c.decodeIfPresent(String.self, forKey: .nodeId) ?? ""
        ok = try c.decodeIfPresent(Bool.self, forKey: .ok) ?? false
        exitCode = try c.decodeIfPresent(Int32.self, forKey: .exitCode) ?? 0
        stdout = try c.decodeIfPresent(String.self, forKey: .stdout) ?? ""
        stderr = try c.decodeIfPresent(String.self, forKey: .stderr) ?? ""
        entries = try c.decodeIfPresent([String].self, forKey: .entries) ?? []
        frameDataUrl = try c.decodeIfPresent(String.self, forKey: .frameDataUrl)
        executedBy = try c.decodeIfPresent(String.self, forKey: .executedBy) ?? ""
        durationMs = try c.decodeIfPresent(UInt64.self, forKey: .durationMs) ?? 0
    }
}

public struct FabushiBotComputerProfileDto: Codable, Sendable, Equatable {
    public let botId: UInt64
    public let boundNodeId: String?
    public let personaPrompt: String
    public let preferredModel: String
    public let approvalMode: FabushiComputerRunPolicy
    public let allowCrossDeviceControl: Bool
    public let allowCloudSelfConfig: Bool
    public let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case botId = "bot_id"
        case boundNodeId = "bound_node_id"
        case personaPrompt = "persona_prompt"
        case preferredModel = "preferred_model"
        case approvalMode = "approval_mode"
        case allowCrossDeviceControl = "allow_cross_device_control"
        case allowCloudSelfConfig = "allow_cloud_self_config"
        case updatedAtMs = "updated_at_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        botId = try c.decodeIfPresent(UInt64.self, forKey: .botId) ?? 0
        boundNodeId = try c.decodeIfPresent(String.self, forKey: .boundNodeId)
        personaPrompt = try c.decodeIfPresent(String.self, forKey: .personaPrompt) ?? ""
        preferredModel = try c.decodeIfPresent(String.self, forKey: .preferredModel) ?? ""
        approvalMode = try c.decodeIfPresent(FabushiComputerRunPolicy.self, forKey: .approvalMode) ?? .askEveryTime
        allowCrossDeviceControl = try c.decodeIfPresent(Bool.self, forKey: .allowCrossDeviceControl) ?? false
        allowCloudSelfConfig = try c.decodeIfPresent(Bool.self, forKey: .allowCloudSelfConfig) ?? false
        updatedAtMs = try c.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct FabushiRemoteAssistConfigDto: Codable, Sendable, Equatable {
    public let assistId: String
    public let allowAssist: Bool
    public let codeMode: FabushiAssistCodeMode
    public let currentCode: String?
    public let expiresAtMs: Int64?

    public init(
        assistId: String = "",
        allowAssist: Bool = false,
        codeMode: FabushiAssistCodeMode = .temporary,
        currentCode: String? = nil,
        expiresAtMs: Int64? = nil
    ) {
        self.assistId = assistId
        self.allowAssist = allowAssist
        self.codeMode = codeMode
        self.currentCode = currentCode
        self.expiresAtMs = expiresAtMs
    }

    enum CodingKeys: String, CodingKey {
        case assistId = "assist_id"
        case allowAssist = "allow_assist"
        case codeMode = "code_mode"
        case currentCode = "current_code"
        case expiresAtMs = "expires_at_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        assistId = try c.decodeIfPresent(String.self, forKey: .assistId) ?? ""
        allowAssist = try c.decodeIfPresent(Bool.self, forKey: .allowAssist) ?? false
        codeMode = try c.decodeIfPresent(FabushiAssistCodeMode.self, forKey: .codeMode) ?? .temporary
        currentCode = try c.decodeIfPresent(String.self, forKey: .currentCode)
        expiresAtMs = try c.decodeIfPresent(Int64.self, forKey: .expiresAtMs)
    }

    public var formattedAssistId: String {
        let digits = assistId.filter { !$0.isWhitespace }
        guard digits.count == 9 else { return assistId }
        let idx3 = digits.index(digits.startIndex, offsetBy: 3)
        let idx6 = digits.index(digits.startIndex, offsetBy: 6)
        return "\(digits[..<idx3]) \(digits[idx3..<idx6]) \(digits[idx6...])"
    }
}

public struct FabushiAssistStartRequest: Codable, Sendable, Equatable {
    public let assistId: String
    public let accessCode: String
    public let requesterDeviceId: String?
    public let requesterName: String?
    public let requesterPlatform: String?
    public let mode: FabushiRemoteControlMode

    public init(
        assistId: String,
        accessCode: String,
        requesterDeviceId: String? = nil,
        requesterName: String? = "Fabushi iOS",
        requesterPlatform: String? = "ios",
        mode: FabushiRemoteControlMode = .desktop
    ) {
        self.assistId = assistId
        self.accessCode = accessCode
        self.requesterDeviceId = requesterDeviceId
        self.requesterName = requesterName
        self.requesterPlatform = requesterPlatform
        self.mode = mode
    }

    enum CodingKeys: String, CodingKey {
        case assistId = "assist_id"
        case accessCode = "access_code"
        case requesterDeviceId = "requester_device_id"
        case requesterName = "requester_name"
        case requesterPlatform = "requester_platform"
        case mode
    }
}

public struct FabushiAssistStartResponse: Codable, Sendable, Equatable {
    public let sessionId: String
    public let targetNodeId: String
    public let targetDeviceId: String
    public let tempRelayToken: String
    public let requiresHostConfirm: Bool
    public let expiresAtMs: Int64?

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case targetNodeId = "target_node_id"
        case targetDeviceId = "target_device_id"
        case tempRelayToken = "temp_relay_token"
        case requiresHostConfirm = "requires_host_confirm"
        case expiresAtMs = "expires_at_ms"
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try c.decodeIfPresent(String.self, forKey: .sessionId) ?? ""
        targetNodeId = try c.decodeIfPresent(String.self, forKey: .targetNodeId) ?? ""
        targetDeviceId = try c.decodeIfPresent(String.self, forKey: .targetDeviceId) ?? ""
        tempRelayToken = try c.decodeIfPresent(String.self, forKey: .tempRelayToken) ?? ""
        requiresHostConfirm = try c.decodeIfPresent(Bool.self, forKey: .requiresHostConfirm) ?? false
        expiresAtMs = try c.decodeIfPresent(Int64.self, forKey: .expiresAtMs)
    }
}

public struct FabushiRemoteSessionDto: Codable, Sendable, Equatable, Identifiable {
    public var id: String { sessionId }
    public let sessionId: String
    public let controllerKind: FabushiRemoteControllerKind
    public let controllerName: String
    public let controllerPlatform: String
    public let controllerId: String
    public let targetNodeId: String
    public let targetDeviceId: String
    public let mode: FabushiRemoteControlMode
    public let startedAtMs: Int64
    public let lastActivityMs: Int64
    public let activeTool: String?
    public let relayMode: FabushiTailnetRelayMode
    public let rttMs: UInt32?
    public let fps: UInt32?

    public init(
        sessionId: String,
        controllerKind: FabushiRemoteControllerKind = .ownDevice,
        controllerName: String,
        controllerPlatform: String = "ios",
        controllerId: String = "",
        targetNodeId: String,
        targetDeviceId: String = "",
        mode: FabushiRemoteControlMode = .desktop,
        startedAtMs: Int64 = 0,
        lastActivityMs: Int64 = 0,
        activeTool: String? = nil,
        relayMode: FabushiTailnetRelayMode = .derpRelay,
        rttMs: UInt32? = nil,
        fps: UInt32? = nil
    ) {
        self.sessionId = sessionId
        self.controllerKind = controllerKind
        self.controllerName = controllerName
        self.controllerPlatform = controllerPlatform
        self.controllerId = controllerId
        self.targetNodeId = targetNodeId
        self.targetDeviceId = targetDeviceId
        self.mode = mode
        self.startedAtMs = startedAtMs
        self.lastActivityMs = lastActivityMs
        self.activeTool = activeTool
        self.relayMode = relayMode
        self.rttMs = rttMs
        self.fps = fps
    }

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case controllerKind = "controller_kind"
        case controllerName = "controller_name"
        case controllerPlatform = "controller_platform"
        case controllerId = "controller_id"
        case targetNodeId = "target_node_id"
        case targetDeviceId = "target_device_id"
        case mode
        case startedAtMs = "started_at_ms"
        case lastActivityMs = "last_activity_ms"
        case activeTool = "active_tool"
        case relayMode = "relay_mode"
        case rttMs = "rtt_ms"
        case fps
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try c.decodeIfPresent(String.self, forKey: .sessionId) ?? ""
        controllerKind = try c.decodeIfPresent(FabushiRemoteControllerKind.self, forKey: .controllerKind) ?? .ownDevice
        controllerName = try c.decodeIfPresent(String.self, forKey: .controllerName) ?? ""
        controllerPlatform = try c.decodeIfPresent(String.self, forKey: .controllerPlatform) ?? ""
        controllerId = try c.decodeIfPresent(String.self, forKey: .controllerId) ?? ""
        targetNodeId = try c.decodeIfPresent(String.self, forKey: .targetNodeId) ?? ""
        targetDeviceId = try c.decodeIfPresent(String.self, forKey: .targetDeviceId) ?? ""
        mode = try c.decodeIfPresent(FabushiRemoteControlMode.self, forKey: .mode) ?? .desktop
        startedAtMs = try c.decodeIfPresent(Int64.self, forKey: .startedAtMs) ?? 0
        lastActivityMs = try c.decodeIfPresent(Int64.self, forKey: .lastActivityMs) ?? 0
        activeTool = try c.decodeIfPresent(String.self, forKey: .activeTool)
        relayMode = try c.decodeIfPresent(FabushiTailnetRelayMode.self, forKey: .relayMode) ?? .httpFallback
        rttMs = try c.decodeIfPresent(UInt32.self, forKey: .rttMs)
        fps = try c.decodeIfPresent(UInt32.self, forKey: .fps)
    }
}

public struct FabushiRemoteSessionsResponse: Codable, Sendable, Equatable {
    public let sessions: [FabushiRemoteSessionDto]

    public init(sessions: [FabushiRemoteSessionDto] = []) {
        self.sessions = sessions
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sessions = try c.decodeIfPresent([FabushiRemoteSessionDto].self, forKey: .sessions) ?? []
    }

    enum CodingKeys: String, CodingKey {
        case sessions
    }
}

public struct FabushiTailnetPeerLiveState: Codable, Sendable, Equatable, Identifiable {
    public var id: String { nodeId }
    public let nodeId: String
    public let deviceId: String
    public let tailnetIp: String
    public let lanIp: String
    public let relayMode: FabushiTailnetRelayMode
    public let activeEndpoint: String?
    public let rttMs: UInt32?
    public let online: Bool

    enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case deviceId = "device_id"
        case tailnetIp = "tailnet_ip"
        case lanIp = "lan_ip"
        case relayMode = "relay_mode"
        case activeEndpoint = "active_endpoint"
        case rttMs = "rtt_ms"
        case online
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nodeId = try c.decodeIfPresent(String.self, forKey: .nodeId) ?? ""
        deviceId = try c.decodeIfPresent(String.self, forKey: .deviceId) ?? ""
        tailnetIp = try c.decodeIfPresent(String.self, forKey: .tailnetIp) ?? ""
        lanIp = try c.decodeIfPresent(String.self, forKey: .lanIp) ?? ""
        relayMode = try c.decodeIfPresent(FabushiTailnetRelayMode.self, forKey: .relayMode) ?? .offline
        activeEndpoint = try c.decodeIfPresent(String.self, forKey: .activeEndpoint)
        rttMs = try c.decodeIfPresent(UInt32.self, forKey: .rttMs)
        online = try c.decodeIfPresent(Bool.self, forKey: .online) ?? false
    }
}

public struct FabushiTailnetNodeStatus: Codable, Sendable, Equatable {
    public let running: Bool
    public let deviceId: String
    public let tailnetIp: String
    public let lanIp: String
    public let machinePublicKey: String
    public let nodePublicKey: String
    public let localUdpPort: UInt16
    public let derpConnected: Bool
    public let relayMode: FabushiTailnetRelayMode
    public let derpRttMs: UInt32?
    public let netmapSeq: UInt64
    public let peers: [FabushiTailnetPeerLiveState]

    public init(
        running: Bool = false,
        deviceId: String = "",
        tailnetIp: String = "",
        lanIp: String = "",
        machinePublicKey: String = "",
        nodePublicKey: String = "",
        localUdpPort: UInt16 = 0,
        derpConnected: Bool = false,
        relayMode: FabushiTailnetRelayMode = .offline,
        derpRttMs: UInt32? = nil,
        netmapSeq: UInt64 = 0,
        peers: [FabushiTailnetPeerLiveState] = []
    ) {
        self.running = running
        self.deviceId = deviceId
        self.tailnetIp = tailnetIp
        self.lanIp = lanIp
        self.machinePublicKey = machinePublicKey
        self.nodePublicKey = nodePublicKey
        self.localUdpPort = localUdpPort
        self.derpConnected = derpConnected
        self.relayMode = relayMode
        self.derpRttMs = derpRttMs
        self.netmapSeq = netmapSeq
        self.peers = peers
    }

    enum CodingKeys: String, CodingKey {
        case running
        case deviceId = "device_id"
        case tailnetIp = "tailnet_ip"
        case lanIp = "lan_ip"
        case machinePublicKey = "machine_public_key"
        case nodePublicKey = "node_public_key"
        case localUdpPort = "local_udp_port"
        case derpConnected = "derp_connected"
        case relayMode = "relay_mode"
        case derpRttMs = "derp_rtt_ms"
        case netmapSeq = "netmap_seq"
        case peers
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        running = try c.decodeIfPresent(Bool.self, forKey: .running) ?? false
        deviceId = try c.decodeIfPresent(String.self, forKey: .deviceId) ?? ""
        tailnetIp = try c.decodeIfPresent(String.self, forKey: .tailnetIp) ?? ""
        lanIp = try c.decodeIfPresent(String.self, forKey: .lanIp) ?? ""
        machinePublicKey = try c.decodeIfPresent(String.self, forKey: .machinePublicKey) ?? ""
        nodePublicKey = try c.decodeIfPresent(String.self, forKey: .nodePublicKey) ?? ""
        localUdpPort = try c.decodeIfPresent(UInt16.self, forKey: .localUdpPort) ?? 0
        derpConnected = try c.decodeIfPresent(Bool.self, forKey: .derpConnected) ?? false
        relayMode = try c.decodeIfPresent(FabushiTailnetRelayMode.self, forKey: .relayMode) ?? .offline
        derpRttMs = try c.decodeIfPresent(UInt32.self, forKey: .derpRttMs)
        netmapSeq = try c.decodeIfPresent(UInt64.self, forKey: .netmapSeq) ?? 0
        peers = try c.decodeIfPresent([FabushiTailnetPeerLiveState].self, forKey: .peers) ?? []
    }
}

public struct FabushiHubEmergencyTakeoverResult: Codable, Sendable, Equatable {
    public let disconnectedSessions: Int
    public let overrideEpoch: UInt64
    public let elapsedUs: UInt64

    enum CodingKeys: String, CodingKey {
        case disconnectedSessions = "disconnected_sessions"
        case overrideEpoch = "override_epoch"
        case elapsedUs = "elapsed_us"
    }
}

public struct FabushiHubPreemptionStatus: Codable, Sendable, Equatable {
    public let overrideEpoch: UInt64
    public let activeCount: Int
    public let sessions: [FabushiRemoteSessionDto]

    enum CodingKeys: String, CodingKey {
        case overrideEpoch = "override_epoch"
        case activeCount = "active_count"
        case sessions
    }
}

public struct FabushiOsc52ClipboardPayload: Codable, Sendable, Equatable {
    public let mimeType: String
    public let text: String?
    public let imageB64: String?
    public let originId: String
    public let seq: UInt64

    enum CodingKeys: String, CodingKey {
        case mimeType = "mime_type"
        case text
        case imageB64 = "image_b64"
        case originId = "origin_id"
        case seq
    }
}

public struct FabushiRemapModifiersResponse: Codable, Sendable, Equatable {
    public let modifiers: [String]
}

// MARK: - UI Navigation & PTY Models

public enum FabushiRemoteHubTab: String, CaseIterable, Identifiable, Sendable {
    case myDevices
    case remoteAssist
    case cloudPCs
    case aiAndMcp

    public var id: String { rawValue }

    public var systemImage: String {
        switch self {
        case .myDevices: return "desktopcomputer"
        case .remoteAssist: return "person.badge.key.fill"
        case .cloudPCs: return "cloud.fill"
        case .aiAndMcp: return "cpu.fill"
        }
    }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .myDevices: return FabushiPlatformStrings.text(.remoteTabMyDevices, locale: locale)
        case .remoteAssist: return FabushiPlatformStrings.text(.remoteTabRemoteAssist, locale: locale)
        case .cloudPCs: return FabushiPlatformStrings.text(.remoteTabCloudPCs, locale: locale)
        case .aiAndMcp: return FabushiPlatformStrings.text(.remoteTabAiMcp, locale: locale)
        }
    }
}

public enum FabushiRemoteWorkbenchTab: String, CaseIterable, Identifiable, Sendable {
    case desktop
    case terminal
    case files
    case settings

    public var id: String { rawValue }

    public var systemImage: String {
        switch self {
        case .desktop: return "display"
        case .terminal: return "terminal.fill"
        case .files: return "folder.fill"
        case .settings: return "slider.horizontal.3"
        }
    }

    public func displayTitle(locale: FabushiLocale) -> String {
        switch self {
        case .desktop: return FabushiPlatformStrings.text(.remoteWorkbenchDesktopTab, locale: locale)
        case .terminal: return FabushiPlatformStrings.text(.remoteWorkbenchTerminalTab, locale: locale)
        case .files: return FabushiPlatformStrings.text(.remoteWorkbenchFilesTab, locale: locale)
        case .settings: return FabushiPlatformStrings.text(.remoteWorkbenchSettingsTab, locale: locale)
        }
    }
}

public struct FabushiPtySessionTab: Identifiable, Sendable, Equatable {
    public let id: String
    public var title: String
    public var sessionName: String
    public var cwd: String
    public var outputLines: [String]

    public init(
        id: String = UUID().uuidString,
        title: String,
        sessionName: String,
        cwd: String = ".",
        outputLines: [String] = []
    ) {
        self.id = id
        self.title = title
        self.sessionName = sessionName
        self.cwd = cwd
        self.outputLines = outputLines
    }
}

// MARK: - Thumbnail Data URL Decoder

public enum FabushiRemoteImageDecoder {
    private static let maxDecodedImageBytes = 12 * 1024 * 1024

    public static func decodeDataURL(_ raw: String?) -> UIImage? {
        guard let raw,
              raw.lowercased().hasPrefix("data:image/"),
              let comma = raw.firstIndex(of: ",")
        else { return nil }
        let header = String(raw[..<comma]).lowercased()
        let payload = String(raw[raw.index(after: comma)...])
        let data: Data?
        if header.hasSuffix(";base64") {
            data = Data(base64Encoded: payload, options: [.ignoreUnknownCharacters])
        } else {
            data = payload.removingPercentEncoding?.data(using: .utf8)
        }
        guard let data, !data.isEmpty, data.count <= maxDecodedImageBytes else {
            return nil
        }
        return UIImage(data: data)
    }
}
