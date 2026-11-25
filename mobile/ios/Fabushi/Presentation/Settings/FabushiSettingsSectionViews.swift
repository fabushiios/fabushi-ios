import Foundation
import Observation
import SwiftUI

// MARK: - Data & Storage (`settings.storageMetrics`, `settings.storagePurge`, `checkpointWAL`)

public struct FabushiStorageMetricsDto: Decodable, Sendable, Equatable {
    public let storageDir: String
    public let totalBytes: UInt64
    public let mediaCasBytes: UInt64
    public let mediaCasFiles: UInt64
    public let sqliteBytes: UInt64
    public let tantivyBytes: UInt64
    public let vaultBytes: UInt64
    public let hermesBytes: UInt64

    enum CodingKeys: String, CodingKey {
        case storageDir = "storage_dir"
        case totalBytes = "total_bytes"
        case mediaCasBytes = "media_cas_bytes"
        case mediaCasFiles = "media_cas_files"
        case sqliteBytes = "sqlite_bytes"
        case tantivyBytes = "tantivy_bytes"
        case vaultBytes = "vault_bytes"
        case hermesBytes = "hermes_bytes"
    }

    public init(
        storageDir: String = "",
        totalBytes: UInt64 = 0,
        mediaCasBytes: UInt64 = 0,
        mediaCasFiles: UInt64 = 0,
        sqliteBytes: UInt64 = 0,
        tantivyBytes: UInt64 = 0,
        vaultBytes: UInt64 = 0,
        hermesBytes: UInt64 = 0
    ) {
        self.storageDir = storageDir
        self.totalBytes = totalBytes
        self.mediaCasBytes = mediaCasBytes
        self.mediaCasFiles = mediaCasFiles
        self.sqliteBytes = sqliteBytes
        self.tantivyBytes = tantivyBytes
        self.vaultBytes = vaultBytes
        self.hermesBytes = hermesBytes
    }
}

public struct FabushiStoragePurgeResultDto: Decodable, Sendable, Equatable {
    public let freedBytes: UInt64
    public let removedFiles: UInt64
    public let metrics: FabushiStorageMetricsDto

    enum CodingKeys: String, CodingKey {
        case freedBytes = "freed_bytes"
        case removedFiles = "removed_files"
        case metrics
    }
}

@MainActor
@Observable
public final class FabushiDataStorageViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    public private(set) var metrics: FabushiStorageMetricsDto = FabushiStorageMetricsDto()
    public private(set) var lastPurgeResult: FabushiStoragePurgeResultDto? = nil
    public private(set) var lastWalCheckpoint: FabushiWalCheckpointSummary? = nil
    public private(set) var isBusy: Bool = false
    public private(set) var lastErrorMessage: String? = nil

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
    }

    public func refreshMetrics() async {
        isBusy = true
        defer { isBusy = false }
        do {
            let fetched: FabushiStorageMetricsDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("storageMetrics"),
                params: FabushiEmptyParams()
            )
            metrics = fetched
            lastErrorMessage = nil
        } catch {
            lastErrorMessage = error.localizedDescription
        }
    }

    @discardableResult
    public func purgeMediaCache(locale: FabushiLocale = .en) async -> FabushiStoragePurgeResultDto? {
        isBusy = true
        defer { isBusy = false }
        do {
            let result: FabushiStoragePurgeResultDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("storagePurge"),
                params: FabushiEmptyParams()
            )
            lastPurgeResult = result
            metrics = result.metrics
            lastErrorMessage = nil
            router.toastController.showNonChatToast(
                "\(FabushiPlatformStrings.text(.settingsStoragePurgedToast, locale: locale)) (\(FabushiPlatformStrings.storageBytesFormatted(result.freedBytes)))",
                isError: false
            )
            return result
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    @discardableResult
    public func runWalCheckpoint(locale: FabushiLocale = .en) async -> FabushiWalCheckpointSummary? {
        isBusy = true
        defer { isBusy = false }
        do {
            let summary = try await coordinator.checkpointWAL()
            lastWalCheckpoint = summary
            await refreshMetrics()
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.settingsStorageCheckpointToast, locale: locale),
                isError: false
            )
            return summary
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }
}

@MainActor
public struct FabushiDataStorageSettingsView: View {
    @State private var viewModel: FabushiDataStorageViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        _viewModel = State(
            initialValue: FabushiDataStorageViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        List {
            Section(FabushiPlatformStrings.text(.settingsStorageOverviewTitle, locale: locale)) {
                Text(
                    "\(FabushiPlatformStrings.storageBytesFormatted(viewModel.metrics.totalBytes)) · \(viewModel.metrics.mediaCasFiles) CAS"
                )
                .fabushiFont(.headline)
                .foregroundStyle(theme.textPrimary)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.storageMetricsLabel)

                metricRow(
                    title: FabushiPlatformStrings.text(.settingsStorageMediaCache, locale: locale),
                    bytes: viewModel.metrics.mediaCasBytes
                )
                metricRow(
                    title: FabushiPlatformStrings.text(.settingsStorageSqliteDb, locale: locale),
                    bytes: viewModel.metrics.sqliteBytes
                )
                metricRow(
                    title: FabushiPlatformStrings.text(.settingsStorageSearchIndex, locale: locale),
                    bytes: viewModel.metrics.tantivyBytes
                )
                metricRow(
                    title: FabushiPlatformStrings.text(.settingsStorageVaultDb, locale: locale),
                    bytes: viewModel.metrics.vaultBytes
                )
                metricRow(
                    title: FabushiPlatformStrings.text(.settingsStorageHermesData, locale: locale),
                    bytes: viewModel.metrics.hermesBytes
                )
            }

            Section {
                Button(role: .destructive) {
                    Task { _ = await viewModel.purgeMediaCache(locale: locale) }
                } label: {
                    HStack {
                        Label(
                            FabushiPlatformStrings.text(.settingsStoragePurgeAction, locale: locale),
                            systemImage: "trash"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.destructiveColor)
                        Spacer()
                        if viewModel.isBusy { ProgressView() }
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(viewModel.isBusy)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.storagePurgeCacheButton)

                Button {
                    Task { _ = await viewModel.runWalCheckpoint(locale: locale) }
                } label: {
                    Label(
                        FabushiPlatformStrings.text(.settingsStorageCheckpointWalAction, locale: locale),
                        systemImage: "cylinder.split.1x2"
                    )
                    .fabushiFont(.body)
                    .foregroundStyle(theme.accentColor)
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(viewModel.isBusy)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionDataStorage, locale: locale))
        .task {
            await viewModel.refreshMetrics()
        }
    }

    private func metricRow(title: String, bytes: UInt64) -> some View {
        HStack {
            Text(title)
                .fabushiFont(.body)
                .foregroundStyle(theme.textPrimary)
            Spacer()
            Text(FabushiPlatformStrings.storageBytesFormatted(bytes))
                .fabushiFont(.monospacedCaption)
                .foregroundStyle(theme.textSecondary)
        }
    }
}

// MARK: - Profile & Blocked Users (`contacts.myProfile`, `contacts.updateProfile`, `contacts.blockedList`, `contacts.unblock`)

public struct FabushiUserProfileDto: Decodable, Sendable, Equatable {
    public let userId: UInt64
    public let username: String?
    public let firstName: String
    public let lastName: String?
    public let bio: String?
    public let phone: String?

    enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case firstName = "first_name"
        case lastName = "last_name"
        case bio
        case phone
    }
}

public struct FabushiBlockedUserDto: Decodable, Identifiable, Sendable, Equatable {
    public let userId: UInt64
    public let username: String?
    public let firstName: String

    public var id: UInt64 { userId }

    enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case username
        case firstName = "first_name"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        userId = try c.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        username = try c.decodeIfPresent(String.self, forKey: .username)
        firstName = try c.decodeIfPresent(String.self, forKey: .firstName) ?? "User"
    }
}

@MainActor
@Observable
public final class FabushiProfileSettingsViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    public var firstName: String = ""
    public var lastName: String = ""
    public var username: String = ""
    public var bio: String = ""
    public private(set) var blockedUsers: [FabushiBlockedUserDto] = []
    public private(set) var isSaving: Bool = false
    public private(set) var lastErrorMessage: String? = nil

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
    }

    public func load() async {
        if let profile: FabushiUserProfileDto = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("myProfile"),
            params: FabushiEmptyParams()
        ) {
            firstName = profile.firstName
            lastName = profile.lastName ?? ""
            username = profile.username ?? ""
            bio = profile.bio ?? ""
        }
        if let blocked: [FabushiBlockedUserDto] = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.contacts.method("blockedList"),
            params: FabushiEmptyParams()
        ) {
            blockedUsers = blocked
        }
    }

    public func saveProfile(locale: FabushiLocale = .en) async {
        isSaving = true
        defer { isSaving = false }

        struct UpdateParams: Encodable, Sendable {
            let first_name: String?
            let last_name: String?
            let username: String?
            let bio: String?
        }

        do {
            let updated: FabushiUserProfileDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.contacts.method("updateProfile"),
                params: UpdateParams(
                    first_name: firstName.isEmpty ? nil : firstName,
                    last_name: lastName.isEmpty ? nil : lastName,
                    username: username.isEmpty ? nil : username,
                    bio: bio
                )
            )
            firstName = updated.firstName
            lastName = updated.lastName ?? ""
            username = updated.username ?? ""
            bio = updated.bio ?? ""
            lastErrorMessage = nil
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.settingsProfileSavedToast, locale: locale),
                isError: false
            )
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func unblock(userId: UInt64) async {
        struct UnblockParams: Encodable, Sendable { let user_id: UInt64 }
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.contacts.method("unblock"),
                params: UnblockParams(user_id: userId)
            )
            blockedUsers.removeAll { $0.userId == userId }
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }
}

@MainActor
public struct FabushiProfileSettingsView: View {
    @State private var viewModel: FabushiProfileSettingsViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        _viewModel = State(
            initialValue: FabushiProfileSettingsViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        List {
            Section(FabushiPlatformStrings.text(.settingsSectionMyProfile, locale: locale)) {
                TextField(
                    FabushiPlatformStrings.text(.settingsProfileFirstName, locale: locale),
                    text: $viewModel.firstName
                )
                .fabushiFont(.body)

                TextField(
                    FabushiPlatformStrings.text(.settingsProfileLastName, locale: locale),
                    text: $viewModel.lastName
                )
                .fabushiFont(.body)

                TextField(
                    FabushiPlatformStrings.text(.settingsProfileUsername, locale: locale),
                    text: $viewModel.username
                )
                .fabushiFont(.body)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()

                TextField(
                    FabushiPlatformStrings.text(.settingsProfileBio, locale: locale),
                    text: $viewModel.bio
                )
                .fabushiFont(.body)

                Button {
                    Task { await viewModel.saveProfile(locale: locale) }
                } label: {
                    Text(FabushiPlatformStrings.text(.settingsProfileSaveAction, locale: locale))
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(viewModel.isSaving)
            }

            Section(FabushiPlatformStrings.text(.settingsBlockedUsersTitle, locale: locale)) {
                if viewModel.blockedUsers.isEmpty {
                    Text(FabushiPlatformStrings.text(.settingsBlockedUsersEmpty, locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(viewModel.blockedUsers) { user in
                        HStack {
                            Text(user.username.map { "@\($0)" } ?? user.firstName)
                                .fabushiFont(.body)
                                .foregroundStyle(theme.textPrimary)
                            Spacer()
                            Button {
                                Task { await viewModel.unblock(userId: user.userId) }
                            } label: {
                                Text(FabushiPlatformStrings.text(.settingsUnblockAction, locale: locale))
                                    .fabushiFont(.caption)
                                    .foregroundStyle(theme.accentColor)
                                    .fabushiMinimumTouchTarget()
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionMyProfile, locale: locale))
        .task {
            await viewModel.load()
        }
    }
}

// MARK: - Local SQLite KV Preferences Section View (Chat Settings, Folders, Stickers, Power Saving)

@MainActor
public struct FabushiLocalPreferencesSettingsView: View {
    public let section: FabushiSettingsSectionType
    public let coordinator: FabushiCoordinator

    @State private var toggleA: Bool = true
    @State private var toggleB: Bool = false

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(section: FabushiSettingsSectionType, coordinator: FabushiCoordinator) {
        self.section = section
        self.coordinator = coordinator
    }

    private var keyA: String { "m5.pref.\(section.rawValue).primary" }
    private var keyB: String { "m5.pref.\(section.rawValue).secondary" }

    private var labelA: String {
        switch section {
        case .chatSettings:
            return FabushiPlatformStrings.text(locale, en: "Send with Return Key (Hardware Keyboard)", zhHans: "外接键盘回车发送")
        case .chatFolders:
            return FabushiPlatformStrings.text(locale, en: "Show Unread Badges on Folders", zhHans: "在会话分组显示未读角标")
        case .stickersAndEmoji:
            return FabushiPlatformStrings.text(locale, en: "Suggest Emoji & Stickers While Typing", zhHans: "输入时智能推荐表情与贴纸")
        case .powerSaving:
            return FabushiPlatformStrings.text(locale, en: "Disable Autoplay Animations on Low Power", zhHans: "低电量时暂停动效自动播放")
        default:
            return FabushiPlatformStrings.text(locale, en: "Enable Feature", zhHans: "启用功能")
        }
    }

    private var labelB: String {
        switch section {
        case .chatSettings:
            return FabushiPlatformStrings.text(locale, en: "Large Emoji Rendering", zhHans: "大号表情渲染")
        case .chatFolders:
            return FabushiPlatformStrings.text(locale, en: "Pin AI & Bot Folder First", zhHans: "置顶 AI 与机器人分组")
        case .stickersAndEmoji:
            return FabushiPlatformStrings.text(locale, en: "Loop Animated Stickers", zhHans: "循环播放动态贴纸")
        case .powerSaving:
            return FabushiPlatformStrings.text(locale, en: "Reduce Background Media Prefetch", zhHans: "减少后台媒体预加载")
        default:
            return FabushiPlatformStrings.text(locale, en: "Compact Mode", zhHans: "紧凑模式")
        }
    }

    public var body: some View {
        List {
            Section(FabushiPlatformStrings.text(section.titleKey, locale: locale)) {
                Toggle(labelA, isOn: $toggleA)
                    .onChange(of: toggleA) { _, newValue in
                        Task { await persist(key: keyA, value: newValue) }
                    }
                Toggle(labelB, isOn: $toggleB)
                    .onChange(of: toggleB) { _, newValue in
                        Task { await persist(key: keyB, value: newValue) }
                    }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(section.titleKey, locale: locale))
        .task {
            await load()
        }
    }

    private func load() async {
        struct LocalGetParams: Encodable, Sendable { let key: String }
        if let resA: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("localGet"),
            params: LocalGetParams(key: keyA)
        ), let valA = resA["value"]?.boolValue {
            toggleA = valA
        }
        if let resB: FabushiJSONValue = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("localGet"),
            params: LocalGetParams(key: keyB)
        ), let valB = resB["value"]?.boolValue {
            toggleB = valB
        }
    }

    private func persist(key: String, value: Bool) async {
        struct LocalSetParams: Encodable, Sendable {
            let key: String
            let value: Bool
        }
        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("localSet"),
            params: LocalSetParams(key: key, value: value)
        ) as FabushiJSONValue
    }
}

// MARK: - Honest Capability Notice Views (Business, Passport/TON BLK-03, Capabilities + BLK-08 Community Foundation)

@MainActor
public struct FabushiCapabilityNoticeSettingsView: View {
    public let titleKey: FabushiPlatformStrings.Key
    public let noticeTitleKey: FabushiPlatformStrings.Key
    public let noticeBodyKey: FabushiPlatformStrings.Key
    public let badgeText: String
    public let systemImage: String

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(
        titleKey: FabushiPlatformStrings.Key,
        noticeTitleKey: FabushiPlatformStrings.Key,
        noticeBodyKey: FabushiPlatformStrings.Key,
        badgeText: String,
        systemImage: String
    ) {
        self.titleKey = titleKey
        self.noticeTitleKey = noticeTitleKey
        self.noticeBodyKey = noticeBodyKey
        self.badgeText = badgeText
        self.systemImage = systemImage
    }

    public var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Label(
                            FabushiPlatformStrings.text(noticeTitleKey, locale: locale),
                            systemImage: systemImage
                        )
                        .fabushiFont(.headline)
                        .foregroundStyle(theme.textPrimary)
                        Spacer()
                        Text(badgeText)
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.warningColor)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .background(theme.surfaceElevated, in: Capsule())
                    }

                    Text(FabushiPlatformStrings.text(noticeBodyKey, locale: locale))
                        .fabushiFont(.callout)
                        .foregroundStyle(theme.textSecondary)
                }
                .padding(.vertical, 6)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(titleKey, locale: locale))
    }
}

/// Canonical Capabilities view delegating to `DefaultCapabilityLedgerView()` PLUS surfacing
/// honest `BLK-08` `D15-04` `UPSTREAM-FOUNDATION` Community disclosure (ZERO fabricated Community UI!).
@MainActor
public struct FabushiCapabilitiesSuiteView: View {
    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init() {}

    public var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Label(
                        FabushiPlatformStrings.text(.settingsCommunityFoundationTitle, locale: locale),
                        systemImage: "building.columns.fill"
                    )
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.textPrimary)

                    Spacer()

                    Text(FabushiPlatformStrings.text(.settingsCommunityFoundationBadge, locale: locale))
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(theme.warningColor)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(theme.surfaceElevated, in: Capsule())
                }

                Text(FabushiPlatformStrings.text(.settingsCommunityFoundationBody, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)
            }
            .padding(14)
            .background(
                theme.surfaceSecondary,
                in: RoundedRectangle(cornerRadius: FabushiLayoutMetrics.cardCornerRadius, style: .continuous)
            )
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("fabushi.settings.communityBlk08Disclosure")

            DefaultCapabilityLedgerView()
        }
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionCapabilities, locale: locale))
    }
}
