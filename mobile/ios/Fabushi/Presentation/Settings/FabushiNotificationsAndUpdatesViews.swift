import Foundation
import Observation
import SwiftUI
import UIKit
import UserNotifications

// MARK: - Notifications & APNs (`UNUserNotificationCenter` + `push.*` + Honest `BLK-01`)

public struct FabushiPrivacyNotificationPreferencesDto: Codable, Sendable, Equatable {
    public var phonePrivacy: String
    public var lastSeenPrivacy: String
    public var photoPrivacy: String
    public var callsPrivacy: String
    public var forwardsPrivacy: String
    public var groupsPrivacy: String
    public var ttlMonths: UInt32
    public var globalTtlSeconds: UInt32
    public var sensitiveContentEnabled: Bool
    public var passcodeEnabled: Bool
    public var notificationsPrivate: Bool
    public var notificationsGroups: Bool
    public var notificationsChannels: Bool
    public var notificationsSound: Bool
    public var includeMutedInBadge: Bool
    public var badgeCountMessages: Bool
    public var p2pMode: String
    public var acceptCalls: Bool
    public var isPremium: Bool

    enum CodingKeys: String, CodingKey {
        case phonePrivacy = "phone_privacy"
        case lastSeenPrivacy = "last_seen_privacy"
        case photoPrivacy = "photo_privacy"
        case callsPrivacy = "calls_privacy"
        case forwardsPrivacy = "forwards_privacy"
        case groupsPrivacy = "groups_privacy"
        case ttlMonths = "ttl_months"
        case globalTtlSeconds = "global_ttl_seconds"
        case sensitiveContentEnabled = "sensitive_content_enabled"
        case passcodeEnabled = "passcode_enabled"
        case notificationsPrivate = "notifications_private"
        case notificationsGroups = "notifications_groups"
        case notificationsChannels = "notifications_channels"
        case notificationsSound = "notifications_sound"
        case includeMutedInBadge = "include_muted_in_badge"
        case badgeCountMessages = "badge_count_messages"
        case p2pMode = "p2p_mode"
        case acceptCalls = "accept_calls"
        case isPremium = "is_premium"
    }

    public init(
        phonePrivacy: String = "Contacts",
        lastSeenPrivacy: String = "Everybody",
        photoPrivacy: String = "Everybody",
        callsPrivacy: String = "Everybody",
        forwardsPrivacy: String = "Everybody",
        groupsPrivacy: String = "Everybody",
        ttlMonths: UInt32 = 6,
        globalTtlSeconds: UInt32 = 0,
        sensitiveContentEnabled: Bool = false,
        passcodeEnabled: Bool = false,
        notificationsPrivate: Bool = true,
        notificationsGroups: Bool = true,
        notificationsChannels: Bool = false,
        notificationsSound: Bool = true,
        includeMutedInBadge: Bool = false,
        badgeCountMessages: Bool = true,
        p2pMode: String = "Contacts",
        acceptCalls: Bool = true,
        isPremium: Bool = false
    ) {
        self.phonePrivacy = phonePrivacy
        self.lastSeenPrivacy = lastSeenPrivacy
        self.photoPrivacy = photoPrivacy
        self.callsPrivacy = callsPrivacy
        self.forwardsPrivacy = forwardsPrivacy
        self.groupsPrivacy = groupsPrivacy
        self.ttlMonths = ttlMonths
        self.globalTtlSeconds = globalTtlSeconds
        self.sensitiveContentEnabled = sensitiveContentEnabled
        self.passcodeEnabled = passcodeEnabled
        self.notificationsPrivate = notificationsPrivate
        self.notificationsGroups = notificationsGroups
        self.notificationsChannels = notificationsChannels
        self.notificationsSound = notificationsSound
        self.includeMutedInBadge = includeMutedInBadge
        self.badgeCountMessages = badgeCountMessages
        self.p2pMode = p2pMode
        self.acceptCalls = acceptCalls
        self.isPremium = isPremium
    }
}

public struct FabushiPushStatusDto: Decodable, Sendable, Equatable {
    public let registered: Bool
    public let serverApnsDispatch: String
    public let blockerId: String

    enum CodingKeys: String, CodingKey {
        case registered
        case serverApnsDispatch = "server_apns_dispatch"
        case blockerId = "blocker_id"
    }

    public init(
        registered: Bool = false,
        serverApnsDispatch: String = "upstream_stub_blocked",
        blockerId: String = "BLK-01"
    ) {
        self.registered = registered
        self.serverApnsDispatch = serverApnsDispatch
        self.blockerId = blockerId
    }
}

@MainActor
@Observable
public final class FabushiNotificationsViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    public var preferences: FabushiPrivacyNotificationPreferencesDto = FabushiPrivacyNotificationPreferencesDto()
    public private(set) var pushStatus: FabushiPushStatusDto = FabushiPushStatusDto()
    public private(set) var authorizationStatusLabel: String = "notDetermined"
    public private(set) var lastErrorMessage: String? = nil

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
    }

    public func refresh() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        authorizationStatusLabel = Self.label(for: settings.authorizationStatus)

        if let status: FabushiPushStatusDto = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.push.method("status"),
            params: FabushiEmptyParams()
        ) {
            pushStatus = status
        }

        if let prefs: FabushiPrivacyNotificationPreferencesDto = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("privacyGet"),
            params: FabushiEmptyParams()
        ) {
            preferences = prefs
        }
    }

    public func requestSystemNotificationPermission() async {
        do {
            let granted = try await UNUserNotificationCenter.current()
                .requestAuthorization(options: [.alert, .badge, .sound])
            if granted {
                UIApplication.shared.registerForRemoteNotifications()
            }
            await refresh()
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func registerCurrentDeviceToken(_ tokenData: Data? = nil, environment: String? = nil) async {
        if let tokenData {
            await coordinator.registerAPNsDeviceToken(tokenData, environment: environment)
        } else if let hex = coordinator.registeredAPNsDeviceTokenHex, !hex.isEmpty {
            struct RegisterParams: Encodable, Sendable {
                let token: String
                let environment: String?
            }
            _ = try? await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.push.method("registerToken"),
                params: RegisterParams(token: hex, environment: environment ?? "production")
            ) as FabushiJSONValue
        } else {
            UIApplication.shared.registerForRemoteNotifications()
        }
        await refresh()
    }

    public func unregisterAPNsToken() async {
        _ = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.push.method("unregisterToken"),
            params: FabushiEmptyParams()
        ) as FabushiJSONValue
        await refresh()
    }

    public func savePreferences() async {
        do {
            let saved: FabushiPrivacyNotificationPreferencesDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("privacySave"),
                params: preferences
            )
            preferences = saved
            lastErrorMessage = nil
        } catch {
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public var formattedApnsStatusSummary: String {
        let regText = pushStatus.registered ? "registered" : "unregistered"
        return "APNs: \(regText) · \(authorizationStatusLabel) · \(pushStatus.serverApnsDispatch) (\(pushStatus.blockerId))"
    }

    private static func label(for status: UNAuthorizationStatus) -> String {
        switch status {
        case .authorized: return "authorized"
        case .denied: return "denied"
        case .provisional: return "provisional"
        case .ephemeral: return "ephemeral"
        case .notDetermined: return "notDetermined"
        @unknown default: return "unknown"
        }
    }
}

@MainActor
public struct FabushiNotificationsSettingsView: View {
    @State private var viewModel: FabushiNotificationsViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        _viewModel = State(
            initialValue: FabushiNotificationsViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        List {
            Section(FabushiPlatformStrings.text(.settingsNotificationsApnsTitle, locale: locale)) {
                Text(viewModel.formattedApnsStatusSummary)
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textPrimary)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.apnsStatusLabel)

                Text(FabushiPlatformStrings.text(.settingsNotificationsBlk01Body, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)

                Button {
                    Task { await viewModel.requestSystemNotificationPermission() }
                } label: {
                    Text(FabushiPlatformStrings.text(.settingsNotificationsRequestPermission, locale: locale))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)

                Button {
                    Task { await viewModel.registerCurrentDeviceToken() }
                } label: {
                    Text(FabushiPlatformStrings.text(.settingsNotificationsRegisterApns, locale: locale))
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)

                if viewModel.pushStatus.registered {
                    Button(role: .destructive) {
                        Task { await viewModel.unregisterAPNsToken() }
                    } label: {
                        Text(FabushiPlatformStrings.text(.settingsNotificationsUnregisterApns, locale: locale))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.destructiveColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }

            Section(FabushiPlatformStrings.text(.settingsSectionNotifications, locale: locale)) {
                Toggle(
                    FabushiPlatformStrings.text(.settingsNotificationsPrivateChats, locale: locale),
                    isOn: $viewModel.preferences.notificationsPrivate
                )
                .onChange(of: viewModel.preferences.notificationsPrivate) { _, _ in
                    Task { await viewModel.savePreferences() }
                }

                Toggle(
                    FabushiPlatformStrings.text(.settingsNotificationsGroups, locale: locale),
                    isOn: $viewModel.preferences.notificationsGroups
                )
                .onChange(of: viewModel.preferences.notificationsGroups) { _, _ in
                    Task { await viewModel.savePreferences() }
                }

                Toggle(
                    FabushiPlatformStrings.text(.settingsNotificationsChannels, locale: locale),
                    isOn: $viewModel.preferences.notificationsChannels
                )
                .onChange(of: viewModel.preferences.notificationsChannels) { _, _ in
                    Task { await viewModel.savePreferences() }
                }

                Toggle(
                    FabushiPlatformStrings.text(.settingsNotificationsSound, locale: locale),
                    isOn: $viewModel.preferences.notificationsSound
                )
                .onChange(of: viewModel.preferences.notificationsSound) { _, _ in
                    Task { await viewModel.savePreferences() }
                }

                Toggle(
                    FabushiPlatformStrings.text(.settingsNotificationsBadgeCount, locale: locale),
                    isOn: $viewModel.preferences.includeMutedInBadge
                )
                .onChange(of: viewModel.preferences.includeMutedInBadge) { _, _ in
                    Task { await viewModel.savePreferences() }
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionNotifications, locale: locale))
        .task {
            await viewModel.refresh()
        }
    }
}

// MARK: - App Updates (`settings.updateCheck`, ZERO Prohibited Binary Self-Updater)

public struct FabushiUpdateCheckResponseDto: Decodable, Sendable, Equatable {
    public let currentVersion: String
    public let distribution: String
    public let selfUpdateSupported: Bool
    public let note: String

    enum CodingKeys: String, CodingKey {
        case currentVersion = "current_version"
        case distribution
        case selfUpdateSupported = "self_update_supported"
        case note
    }

    public init(
        currentVersion: String = "1.6.15",
        distribution: String = "app_store_testflight",
        selfUpdateSupported: Bool = false,
        note: String = "iOS updates are delivered strictly through App Store / TestFlight"
    ) {
        self.currentVersion = currentVersion
        self.distribution = distribution
        self.selfUpdateSupported = selfUpdateSupported
        self.note = note
    }
}

@MainActor
@Observable
public final class FabushiAppUpdatesViewModel {
    public let coordinator: FabushiCoordinator

    public private(set) var updateInfo: FabushiUpdateCheckResponseDto? = nil
    public private(set) var isChecking: Bool = false
    public private(set) var lastErrorMessage: String? = nil

    public init(coordinator: FabushiCoordinator) {
        self.coordinator = coordinator
    }

    @discardableResult
    public func checkForUpdates() async -> FabushiUpdateCheckResponseDto? {
        isChecking = true
        defer { isChecking = false }
        do {
            let res: FabushiUpdateCheckResponseDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.settings.method("updateCheck"),
                params: FabushiEmptyParams()
            )
            updateInfo = res
            lastErrorMessage = nil
            return res
        } catch {
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }
}

@MainActor
public struct FabushiAppUpdatesSettingsView: View {
    @State private var viewModel: FabushiAppUpdatesViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator) {
        _viewModel = State(initialValue: FabushiAppUpdatesViewModel(coordinator: coordinator))
    }

    public var body: some View {
        List {
            Section(FabushiPlatformStrings.text(.settingsUpdatesTitle, locale: locale)) {
                let statusText: String = {
                    if let info = viewModel.updateInfo {
                        return "v\(info.currentVersion) · \(info.distribution) · self_update=\(info.selfUpdateSupported)"
                    }
                    return FabushiPlatformStrings.text(.settingsUpdatesAppStoreNote, locale: locale)
                }()

                Text(statusText)
                    .fabushiFont(.monospacedCaption)
                    .foregroundStyle(theme.textPrimary)
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.updateStatusLabel)

                Text(FabushiPlatformStrings.text(.settingsUpdatesAppStoreNote, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.textSecondary)

                Button {
                    Task { _ = await viewModel.checkForUpdates() }
                } label: {
                    HStack {
                        Text(FabushiPlatformStrings.text(.settingsUpdatesCheckAction, locale: locale))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.accentColor)
                        Spacer()
                        if viewModel.isChecking {
                            ProgressView()
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(viewModel.isChecking)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.updateCheckButton)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionAppUpdates, locale: locale))
        .task {
            _ = await viewModel.checkForUpdates()
        }
    }
}

// MARK: - Typed Action Bus Inspector (`actions.list`, `actions.listByCategory`, `actions.execute`)

public struct FabushiActionBusItemDto: Decodable, Identifiable, Sendable, Equatable {
    public let name: String
    public let description: String
    public let category: String
    public let riskLevel: String

    public var id: String { name }

    enum CodingKeys: String, CodingKey {
        case name
        case description
        case category
        case riskLevel = "risk_level"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        category = try c.decodeIfPresent(String.self, forKey: .category) ?? "System"
        riskLevel = try c.decodeIfPresent(String.self, forKey: .riskLevel) ?? "Low"
    }
}

public struct FabushiActionBusListResponseDto: Decodable, Sendable, Equatable {
    public let count: Int
    public let actions: [FabushiActionBusItemDto]
}

@MainActor
@Observable
public final class FabushiActionBusViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    public private(set) var actions: [FabushiActionBusItemDto] = []
    public private(set) var lastExecutionSummary: String? = nil
    public private(set) var lastExecutionWasSandboxBlocked: Bool = false
    public private(set) var isLoading: Bool = false

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        self.coordinator = coordinator
        self.router = router
    }

    public func loadActions(category: String? = nil) async {
        isLoading = true
        defer { isLoading = false }
        do {
            let res: FabushiActionBusListResponseDto
            if let category, !category.isEmpty {
                struct CategoryParams: Encodable, Sendable { let category: String }
                res = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.actions.method("listByCategory"),
                    params: CategoryParams(category: category)
                )
            } else {
                res = try await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.actions.method("list"),
                    params: FabushiEmptyParams()
                )
            }
            actions = res.actions
        } catch {
            lastExecutionSummary = error.localizedDescription
        }
    }

    @discardableResult
    public func executeAction(
        name: String,
        input: FabushiJSONValue = .object([:]),
        approved: Bool = false
    ) async -> Bool {
        struct ExecuteParams: Encodable, Sendable {
            let name: String
            let input: FabushiJSONValue
            let approved: Bool
        }

        lastExecutionWasSandboxBlocked = false
        do {
            let result: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.actions.method("execute"),
                params: ExecuteParams(name: name, input: input, approved: approved)
            )
            lastExecutionSummary = "\(name): \(result.toJSONString())"
            router.toastController.showNonChatToast("\(name) completed", isError: false)
            return true
        } catch let hostError as FabushiHostError {
            if case .capabilityUnavailable = hostError {
                lastExecutionWasSandboxBlocked = true
            }
            lastExecutionSummary = "\(name) [\(hostError.code)]: \(hostError.localizedDescription)"
            router.toastController.showNonChatToast(hostError.localizedDescription, isError: true)
            return false
        } catch {
            lastExecutionSummary = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }
}

@MainActor
public struct FabushiActionBusSettingsView: View {
    @State private var viewModel: FabushiActionBusViewModel

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        _viewModel = State(
            initialValue: FabushiActionBusViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        List {
            Section(FabushiPlatformStrings.text(.settingsActionBusTitle, locale: locale)) {
                Text(FabushiPlatformStrings.text(.settingsActionBusSiriNote, locale: locale))
                    .fabushiFont(.footnote)
                    .foregroundStyle(theme.textPrimary)

                Text(FabushiPlatformStrings.text(.settingsActionBusSandboxNotice, locale: locale))
                    .fabushiFont(.caption)
                    .foregroundStyle(theme.warningColor)

                if let summary = viewModel.lastExecutionSummary {
                    Text(summary)
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(viewModel.lastExecutionWasSandboxBlocked ? theme.warningColor : theme.textSecondary)
                }
            }

            Section {
                ForEach(viewModel.actions) { item in
                    HStack(spacing: 10) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.name)
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textPrimary)
                            Text("\(item.category) · \(item.riskLevel)")
                                .fabushiFont(.caption2)
                                .foregroundStyle(theme.textSecondary)
                        }
                        Spacer()
                        Button {
                            Task { _ = await viewModel.executeAction(name: item.name) }
                        } label: {
                            Text(FabushiPlatformStrings.text(.settingsActionBusRunAction, locale: locale))
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.accentColor)
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.settingsSectionActionBusIntents, locale: locale))
        .task {
            await viewModel.loadActions()
        }
    }
}
