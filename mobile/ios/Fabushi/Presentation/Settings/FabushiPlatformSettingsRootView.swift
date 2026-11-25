import SwiftUI

// MARK: - Complete 26-Section Settings Taxonomy (`Feature #30`)

public enum FabushiSettingsGroup: String, CaseIterable, Sendable, Identifiable {
    case account
    case experience
    case securityAndData
    case aiAndPlatform
    case economyAndSystem

    public var id: String { rawValue }

    public var titleKey: FabushiPlatformStrings.Key {
        switch self {
        case .account: return .settingsGroupAccount
        case .experience: return .settingsGroupExperience
        case .securityAndData: return .settingsGroupSecurityAndData
        case .aiAndPlatform: return .settingsGroupAiAndPlatform
        case .economyAndSystem: return .settingsGroupEconomyAndSystem
        }
    }
}

/// Complete 26-section Apple Settings Suite enum (`CaseIterable`, `allCases.count == 26`).
/// Distinct from frozen `FabushiSettingsSectionDestination` (7-case M2 enum) to avoid any symbol collision.
public enum FabushiSettingsSectionType: String, CaseIterable, Hashable, Sendable, Identifiable {
    // 1. Account & Profile (2)
    case myProfile
    case accounts

    // 2. Appearance & Messaging (6)
    case appearance
    case language
    case chatSettings
    case chatFolders
    case stickersAndEmoji
    case notifications

    // 3. Privacy, Security & Network (6)
    case privacySecurity
    case devices
    case credentialVault
    case dataStorage
    case networkProxy
    case powerSaving

    // 4. AI, Remote & Platform (6)
    case aiProviders
    case officialMcp
    case remoteComputers
    case miniApps
    case builtInBrowser
    case callsAndTalk

    // 5. Economy, Updates & Capabilities (6)
    case walletStars
    case business
    case passportAndTon
    case actionBusIntents
    case appUpdates
    case capabilities

    public var id: String { rawValue }

    public var group: FabushiSettingsGroup {
        switch self {
        case .myProfile, .accounts:
            return .account
        case .appearance, .language, .chatSettings, .chatFolders, .stickersAndEmoji, .notifications:
            return .experience
        case .privacySecurity, .devices, .credentialVault, .dataStorage, .networkProxy, .powerSaving:
            return .securityAndData
        case .aiProviders, .officialMcp, .remoteComputers, .miniApps, .builtInBrowser, .callsAndTalk:
            return .aiAndPlatform
        case .walletStars, .business, .passportAndTon, .actionBusIntents, .appUpdates, .capabilities:
            return .economyAndSystem
        }
    }

    public var titleKey: FabushiPlatformStrings.Key {
        switch self {
        case .myProfile: return .settingsSectionMyProfile
        case .accounts: return .settingsSectionAccounts
        case .appearance: return .settingsSectionAppearance
        case .language: return .settingsSectionLanguage
        case .chatSettings: return .settingsSectionChatSettings
        case .chatFolders: return .settingsSectionChatFolders
        case .stickersAndEmoji: return .settingsSectionStickersEmoji
        case .notifications: return .settingsSectionNotifications
        case .privacySecurity: return .settingsSectionPrivacySecurity
        case .devices: return .settingsSectionDevices
        case .credentialVault: return .settingsSectionCredentialVault
        case .dataStorage: return .settingsSectionDataStorage
        case .networkProxy: return .settingsSectionNetworkProxy
        case .powerSaving: return .settingsSectionPowerSaving
        case .aiProviders: return .settingsSectionAiProviders
        case .officialMcp: return .settingsSectionOfficialMcp
        case .remoteComputers: return .settingsSectionRemoteComputers
        case .miniApps: return .settingsSectionMiniApps
        case .builtInBrowser: return .settingsSectionBuiltInBrowser
        case .callsAndTalk: return .settingsSectionCallsAndTalk
        case .walletStars: return .settingsSectionWalletStars
        case .business: return .settingsSectionBusiness
        case .passportAndTon: return .settingsSectionPassportTon
        case .actionBusIntents: return .settingsSectionActionBusIntents
        case .appUpdates: return .settingsSectionAppUpdates
        case .capabilities: return .settingsSectionCapabilities
        }
    }

    public var systemImage: String {
        switch self {
        case .myProfile: return "person.crop.circle.fill"
        case .accounts: return "person.2.circle.fill"
        case .appearance: return "paintbrush.fill"
        case .language: return "globe"
        case .chatSettings: return "bubble.left.and.bubble.right.fill"
        case .chatFolders: return "folder.fill"
        case .stickersAndEmoji: return "face.smiling.fill"
        case .notifications: return "bell.badge.fill"
        case .privacySecurity: return "lock.shield.fill"
        case .devices: return "laptopcomputer.and.iphone"
        case .credentialVault: return "key.fill"
        case .dataStorage: return "internaldrive.fill"
        case .networkProxy: return "network"
        case .powerSaving: return "battery.75percent"
        case .aiProviders: return "cpu"
        case .officialMcp: return "server.rack"
        case .remoteComputers: return "desktopcomputer"
        case .miniApps: return "square.grid.2x2.fill"
        case .builtInBrowser: return "safari.fill"
        case .callsAndTalk: return "waveform.and.mic"
        case .walletStars: return "star.circle.fill"
        case .business: return "briefcase.fill"
        case .passportAndTon: return "checkmark.shield.fill"
        case .actionBusIntents: return "bolt.circle.fill"
        case .appUpdates: return "arrow.down.app.fill"
        case .capabilities: return "checkmark.seal.fill"
        }
    }

    /// Maps ALL 13 canonical `FabushiAccessibilityIDs.Settings.section*` identifiers cleanly.
    public var accessibilityID: String {
        switch self {
        case .appearance: return FabushiAccessibilityIDs.Settings.sectionAppearance
        case .language: return FabushiAccessibilityIDs.Settings.sectionLanguage
        case .privacySecurity: return FabushiAccessibilityIDs.Settings.sectionPrivacySecurity
        case .devices: return FabushiAccessibilityIDs.Settings.sectionDevices
        case .credentialVault: return FabushiAccessibilityIDs.Settings.sectionVault
        case .dataStorage: return FabushiAccessibilityIDs.Settings.sectionDataStorage
        case .networkProxy: return FabushiAccessibilityIDs.Settings.sectionNetworkProxy
        case .aiProviders: return FabushiAccessibilityIDs.Settings.sectionAIProviders
        case .officialMcp: return FabushiAccessibilityIDs.Settings.sectionMCP
        case .walletStars: return FabushiAccessibilityIDs.Settings.sectionWalletStars
        case .notifications: return FabushiAccessibilityIDs.Settings.sectionNotifications
        case .appUpdates: return FabushiAccessibilityIDs.Settings.sectionUpdates
        case .capabilities: return FabushiAccessibilityIDs.Settings.sectionCapabilities
        default: return "fabushi.settings.section.\(rawValue)"
        }
    }

    public var searchKeywords: [String] {
        switch self {
        case .myProfile: return ["profile", "account", "bio", "username", "blocked", "个人", "资料", "黑名单"]
        case .accounts: return ["accounts", "switch", "multi", "多账号", "切换"]
        case .appearance: return ["appearance", "theme", "night", "tinted", "day", "classic", "accent", "外观", "主题", "配色"]
        case .language: return ["language", "locale", "english", "chinese", "zh", "语言", "中文", "英文"]
        case .chatSettings: return ["chat", "messages", "enter", "return", "聊天", "消息"]
        case .chatFolders: return ["folders", "tabs", "分组", "文件夹"]
        case .stickersAndEmoji: return ["stickers", "emoji", "reactions", "贴纸", "表情"]
        case .notifications: return ["notifications", "push", "apns", "sound", "badge", "blk-01", "通知", "推送", "声音"]
        case .privacySecurity: return ["privacy", "security", "passcode", "2fa", "faceid", "隐私", "安全", "锁屏"]
        case .devices: return ["devices", "sessions", "qr", "设备", "会话"]
        case .credentialVault: return ["vault", "totp", "passwords", "credentials", "凭证库", "密钥"]
        case .dataStorage: return ["data", "storage", "cache", "sqlite", "wal", "purge", "存储", "缓存", "清理"]
        case .networkProxy: return ["network", "proxy", "socks5", "http", "mtproto", "server", "url", "tg://", "网络", "代理", "服务器"]
        case .powerSaving: return ["power", "battery", "animations", "performance", "省电", "性能"]
        case .aiProviders: return ["ai", "providers", "magpie", "models", "llm", "oauth", "模型", "供应商"]
        case .officialMcp: return ["mcp", "tools", "audit", "oauth", "控制台", "工具"]
        case .remoteComputers: return ["remote", "computers", "tailnet", "pty", "uu", "远程", "电脑", "终端"]
        case .miniApps: return ["miniapp", "mini apps", "webmcp", "dharma", "global dharma", "小程序", "法布施"]
        case .builtInBrowser: return ["browser", "automation", "store", "wkwebview", "recorder", "浏览器", "自动化"]
        case .callsAndTalk: return ["calls", "voice", "video", "webrtc", "talk", "orb", "vad", "通话", "语音", "视频"]
        case .walletStars: return ["wallet", "stars", "gifts", "premium", "storekit", "redeem", "blk-04", "钱包", "星币", "会员"]
        case .business: return ["business", "greeting", "hours", "商业", "营业"]
        case .passportAndTon: return ["passport", "ton", "blockchain", "blk-03", "护照", "链上"]
        case .actionBusIntents: return ["actions", "action bus", "siri", "shortcuts", "intents", "动作总线", "快捷指令"]
        case .appUpdates: return ["updates", "version", "testflight", "app store", "更新", "版本"]
        case .capabilities: return ["capabilities", "ledger", "blk", "community", "ocrm", "fbcp", "upstream", "能力", "社区", "清单"]
        }
    }

    public static func sections(in group: FabushiSettingsGroup) -> [FabushiSettingsSectionType] {
        allCases.filter { $0.group == group }
    }

    /// Filters ALL 26 sections across BOTH `en` and `zh-Hans` localized titles AND bilingual keywords!
    public static func matching(query rawQuery: String) -> [FabushiSettingsSectionType] {
        let query = rawQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return allCases }
        return allCases.filter { section in
            let enTitle = FabushiPlatformStrings.text(section.titleKey, locale: .en).lowercased()
            let zhTitle = FabushiPlatformStrings.text(section.titleKey, locale: .zhHans).lowercased()
            if enTitle.contains(query) || zhTitle.contains(query) || section.rawValue.lowercased().contains(query) {
                return true
            }
            return section.searchKeywords.contains { $0.lowercased().contains(query) }
        }
    }

    public static func parse(_ raw: String?) -> FabushiSettingsSectionType? {
        guard let trimmed = raw?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else {
            return nil
        }
        let normalized = trimmed
            .lowercased()
            .replacingOccurrences(of: "-", with: "")
            .replacingOccurrences(of: "_", with: "")
            .replacingOccurrences(of: " ", with: "")

        switch normalized {
        case "profile", "myprofile", "me": return .myProfile
        case "accounts", "account", "switcher": return .accounts
        case "appearance", "theme", "themes", "display", "accent": return .appearance
        case "language", "lang", "locale", "locales": return .language
        case "chat", "chatsettings": return .chatSettings
        case "folders", "chatfolders": return .chatFolders
        case "stickers", "emoji", "stickersandemoji": return .stickersAndEmoji
        case "notifications", "push", "apns", "sounds": return .notifications
        case "privacy", "security", "privacysecurity", "passcode": return .privacySecurity
        case "devices", "device", "sessions", "activesessions": return .devices
        case "vault", "credentialvault", "credentials", "totp": return .credentialVault
        case "storage", "datastorage", "data", "cache": return .dataStorage
        case "network", "proxy", "networkproxy", "server": return .networkProxy
        case "power", "powersaving", "battery": return .powerSaving
        case "ai", "aiproviders", "aiprovider", "providers", "magpie", "models": return .aiProviders
        case "mcp", "officialmcp": return .officialMcp
        case "remote", "computers", "remotecomputers": return .remoteComputers
        case "miniapps", "miniapp", "globaldharma", "dharma": return .miniApps
        case "browser", "builtinbrowser", "automation": return .builtInBrowser
        case "calls", "talk", "callsandsound", "callsandtalk": return .callsAndTalk
        case "wallet", "stars", "walletstars", "premium", "economy", "storekit": return .walletStars
        case "business": return .business
        case "passport", "ton", "passportandton": return .passportAndTon
        case "actions", "actionbus", "actionbusintents", "shortcuts", "intents": return .actionBusIntents
        case "updates", "appupdates", "update", "version": return .appUpdates
        case "capabilities", "capability", "ledger", "capabilityledger", "community": return .capabilities
        default: return nil
        }
    }
}

// MARK: - Explicit Back Button Modifier (`FabushiAccessibilityIDs.Shell.backButton`)

@MainActor
private struct FabushiPlatformSettingsBackButtonModifier: ViewModifier {
    let onBack: () -> Void
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }

    func body(content: Content) -> some View {
        content
            .navigationBarBackButtonHidden(true)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(action: onBack) {
                        HStack(spacing: 4) {
                            Image(systemName: "chevron.backward")
                                .fabushiFont(.headline)
                                .accessibilityHidden(true)
                            Text(theme.localized("hig.common.back"))
                                .fabushiFont(.body)
                                .lineLimit(1)
                        }
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized("hig.common.back"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.backButton)
                }
            }
    }
}

// MARK: - Full 26-Section Apple HIG Settings Root View

/// Complete 26-section Settings root view returned by `PlatformModulesProviding.makeSettingsRootView`.
///
/// - NO nested `NavigationStack` at root (shell already wraps `makeSettingsRootView` in `NavigationStack`).
/// - Adaptive iPhone `List(.insetGrouped)` push navigation + iPad side-by-side master-detail split.
/// - Live bilingual search across all 26 sections (`FabushiAccessibilityIDs.Settings.searchField`).
/// - Delegates M2/M4 sections directly to their canonical owners (`FabushiAppearanceSettingsView`,
///   `FabushiLanguageSettingsView`, `registry.security`, `registry.agentStudio`, `DefaultCapabilityLedgerView`).
@MainActor
public struct FabushiPlatformSettingsRootView: View {
    public let initialSection: String?
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @Environment(FabushiFeatureModuleRegistry.self) private var injectedRegistry: FabushiFeatureModuleRegistry?
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var searchQuery: String = ""
    @State private var selectedSection: FabushiSettingsSectionType? = nil
    @State private var isAccountSwitcherPresented: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var registry: FabushiFeatureModuleRegistry { injectedRegistry ?? .shared }
    private var locale: FabushiLocale { theme.locale }
    private var isRegularWidth: Bool { horizontalSizeClass == .regular }

    public init(
        initialSection: String? = nil,
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter
    ) {
        self.initialSection = initialSection
        self.coordinator = coordinator
        self.router = router
    }

    public var body: some View {
        Group {
            if isRegularWidth {
                HStack(spacing: 0) {
                    sectionListColumn
                        .frame(minWidth: 290, idealWidth: 320, maxWidth: 360)
                    Divider()
                    if let active = selectedSection {
                        destinationView(for: active)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        destinationView(for: .appearance)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
            } else {
                sectionListColumn
                    .navigationDestination(item: $selectedSection) { section in
                        destinationView(for: section)
                            .modifier(
                                FabushiPlatformSettingsBackButtonModifier {
                                    selectedSection = nil
                                    if case .settings(let currentSection) = router.activeRoute, currentSection != nil {
                                        router.navigate(to: .settings(section: nil), coordinator: coordinator)
                                    }
                                }
                            )
                    }
            }
        }
        .background(theme.surfacePrimary)
        .navigationTitle(theme.localized("hig.tab.settings"))
        .onAppear {
            if let parsed = FabushiSettingsSectionType.parse(initialSection) {
                selectedSection = parsed
            }
        }
        .onChange(of: initialSection) { _, newValue in
            selectedSection = FabushiSettingsSectionType.parse(newValue)
        }
        .sheet(isPresented: $isAccountSwitcherPresented) {
            registry.security.makeAccountSwitcherSheet(
                coordinator: coordinator,
                router: router,
                onDismiss: { isAccountSwitcherPresented = false }
            )
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.root)
    }

    private var sectionListColumn: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(theme.textSecondary)
                    .accessibilityHidden(true)

                TextField(
                    theme.localized("hig.settings.searchPlaceholder"),
                    text: $searchQuery
                )
                .fabushiFont(.body)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityLabel(theme.localized("hig.settings.searchPlaceholder"))
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.searchField)

                if !searchQuery.isEmpty {
                    Button {
                        searchQuery = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(theme.textMuted)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                theme.surfaceInput,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.searchCornerRadius, style: .continuous)
            )
            .padding(.horizontal, 16)
            .padding(.vertical, 8)

            let filtered = FabushiSettingsSectionType.matching(query: searchQuery)

            List {
                if filtered.isEmpty {
                    Text(FabushiPlatformStrings.text(.settingsNoMatchingSections, locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(FabushiSettingsGroup.allCases) { group in
                        let items = filtered.filter { $0.group == group }
                        if !items.isEmpty {
                            Section(FabushiPlatformStrings.text(group.titleKey, locale: locale)) {
                                ForEach(items) { section in
                                    settingsRow(for: section)
                                }
                            }
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.sectionList)
        }
    }

    private func settingsRow(for section: FabushiSettingsSectionType) -> some View {
        let title = FabushiPlatformStrings.text(section.titleKey, locale: locale)
        return Button {
            handleSelection(section)
        } label: {
            HStack(spacing: 12) {
                Label(title, systemImage: section.systemImage)
                    .fabushiFont(.body)
                    .foregroundStyle(theme.textPrimary)
                Spacer(minLength: 8)
                Image(systemName: "chevron.forward")
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textMuted)
                    .accessibilityHidden(true)
            }
            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityIdentifier(section.accessibilityID)
    }

    private func handleSelection(_ section: FabushiSettingsSectionType) {
        switch section {
        case .accounts:
            isAccountSwitcherPresented = true
        case .miniApps:
            router.navigate(to: .miniApp(id: "catalog"), coordinator: coordinator)
        case .callsAndTalk:
            router.navigate(to: .calls, coordinator: coordinator)
        default:
            selectedSection = section
        }
    }

    @ViewBuilder
    private func destinationView(for section: FabushiSettingsSectionType) -> some View {
        switch section {
        case .myProfile:
            FabushiProfileSettingsView(coordinator: coordinator, router: router)
        case .accounts:
            registry.security.makeAccountSwitcherSheet(
                coordinator: coordinator,
                router: router,
                onDismiss: { selectedSection = nil }
            )
        case .appearance:
            FabushiAppearanceSettingsView(coordinator: coordinator)
        case .language:
            FabushiLanguageSettingsView(coordinator: coordinator)
        case .chatSettings, .chatFolders, .stickersAndEmoji, .powerSaving:
            FabushiLocalPreferencesSettingsView(section: section, coordinator: coordinator)
        case .notifications:
            FabushiNotificationsSettingsView(coordinator: coordinator, router: router)
        case .privacySecurity:
            registry.security.makePrivacySecuritySettingsView(coordinator: coordinator, router: router)
        case .devices:
            registry.security.makeDevicesSettingsView(coordinator: coordinator)
        case .credentialVault:
            registry.security.makeVaultSettingsView(coordinator: coordinator)
        case .dataStorage:
            FabushiDataStorageSettingsView(coordinator: coordinator, router: router)
        case .networkProxy:
            FabushiNetworkProxySettingsView(coordinator: coordinator, router: router)
        case .aiProviders:
            registry.agentStudio.makeMagpieAiProvidersSettingsView(coordinator: coordinator, router: router)
        case .officialMcp:
            registry.agentStudio.makeOfficialMcpConsoleView(coordinator: coordinator, router: router)
        case .remoteComputers:
            FabushiRemoteComputerHubView(nodeId: nil, coordinator: coordinator, router: router)
        case .miniApps:
            FabushiMiniAppHostSheetView(
                miniAppId: "catalog",
                coordinator: coordinator,
                router: router,
                sessionPool: .shared
            )
        case .builtInBrowser:
            FabushiBrowserWorkspaceView(
                initialURL: nil,
                coordinator: coordinator,
                router: router
            )
        case .callsAndTalk:
            FabushiCallsTabView(coordinator: coordinator, router: router)
        case .walletStars:
            FabushiWalletAndPremiumView(coordinator: coordinator, router: router)
        case .business:
            FabushiCapabilityNoticeSettingsView(
                titleKey: .settingsSectionBusiness,
                noticeTitleKey: .settingsBusinessNoticeTitle,
                noticeBodyKey: .settingsBusinessNoticeBody,
                badgeText: "PARTIAL",
                systemImage: "briefcase.fill"
            )
        case .passportAndTon:
            FabushiCapabilityNoticeSettingsView(
                titleKey: .settingsSectionPassportTon,
                noticeTitleKey: .settingsPassportTonNoticeTitle,
                noticeBodyKey: .settingsPassportTonNoticeBody,
                badgeText: "BLK-03",
                systemImage: "checkmark.shield.fill"
            )
        case .actionBusIntents:
            FabushiActionBusSettingsView(coordinator: coordinator, router: router)
        case .appUpdates:
            FabushiAppUpdatesSettingsView(coordinator: coordinator)
        case .capabilities:
            FabushiCapabilitiesSuiteView()
        }
    }
}
