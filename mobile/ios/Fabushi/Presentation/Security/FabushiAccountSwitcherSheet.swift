import SwiftUI

// MARK: - Multi-Account Quota Policy (3 Standard / 6 Premium — Desktop v1.6.9 Parity)

public enum FabushiAccountQuotaPolicy: Sendable {
    public static let standardLimit: Int = 3
    public static let premiumLimit: Int = 6

    public static func maxAccounts(isPremium: Bool) -> Int {
        isPremium ? premiumLimit : standardLimit
    }

    public static func canAddAccount(currentCount: Int, isPremium: Bool) -> Bool {
        currentCount < maxAccounts(isPremium: isPremium)
    }
}

// MARK: - Multi-Account Switcher Sheet (F06)

@MainActor
public struct FabushiAccountSwitcherSheet: View {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter
    public let securityKeychain: FabushiSecurityKeychainStore
    public let onDismiss: () -> Void

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    @State private var accounts: [FabushiAccountSummary] = []
    @State private var isPremiumAccount: Bool = false
    @State private var isLoading: Bool = false
    @State private var isMutating: Bool = false
    @State private var errorMessage: String?
    @State private var showingAddAccount: Bool = false

    private var theme: FabushiTheme { injectedTheme ?? .default }

    private struct MembershipResponse: Decodable, Sendable {
        let isPremium: Bool

        enum CodingKeys: String, CodingKey {
            case isPremium = "is_premium"
        }
    }

    private struct PrivacyPremiumResponse: Decodable, Sendable {
        let isPremium: Bool

        enum CodingKeys: String, CodingKey {
            case isPremium = "is_premium"
        }
    }

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        securityKeychain: FabushiSecurityKeychainStore = .shared,
        onDismiss: @escaping () -> Void
    ) {
        self.coordinator = coordinator
        self.router = router
        self.securityKeychain = securityKeychain
        self.onDismiss = onDismiss
    }

    private var maxQuota: Int {
        FabushiAccountQuotaPolicy.maxAccounts(isPremium: isPremiumAccount)
    }

    public var body: some View {
        NavigationStack {
            List {
                Section {
                    HStack(spacing: 10) {
                        Image(systemName: isPremiumAccount ? "star.circle.fill" : "person.2.circle.fill")
                            .fabushiFont(.title3)
                            .foregroundStyle(theme.accentColor)
                            .accessibilityHidden(true)

                        VStack(alignment: .leading, spacing: 2) {
                            Text(
                                isPremiumAccount
                                    ? theme.localized("hig.accountSwitcher.quotaPremium")
                                    : theme.localized("hig.accountSwitcher.quotaStandard")
                            )
                            .fabushiFont(.subheadline)
                            .foregroundStyle(theme.textPrimary)

                            Text("\(accounts.count) / \(maxQuota)")
                                .fabushiFont(.footnote)
                                .foregroundStyle(theme.textSecondary)
                        }
                    }
                    .padding(.vertical, 4)
                }

                Section(header: Text(theme.localized("hig.accountSwitcher.signedInHeader"))) {
                    if isLoading && accounts.isEmpty {
                        ProgressView()
                            .frame(maxWidth: .infinity, alignment: .center)
                    } else if accounts.isEmpty {
                        Text(theme.localized("hig.accountSwitcher.empty"))
                            .fabushiFont(.body)
                            .foregroundStyle(theme.textSecondary)
                    } else {
                        ForEach(Array(accounts.enumerated()), id: \.element.accountId) { idx, account in
                            accountRow(account: account, index: idx)
                        }
                    }
                }

                if let errorMessage {
                    Section {
                        Text(errorMessage)
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.destructiveColor)
                            .accessibilityLabel(errorMessage)
                    }
                }

                Section {
                    Button {
                        if FabushiAccountQuotaPolicy.canAddAccount(
                            currentCount: accounts.count,
                            isPremium: isPremiumAccount
                        ) {
                            errorMessage = nil
                            showingAddAccount = true
                        } else {
                            errorMessage = theme.localized("hig.accountSwitcher.quotaReached")
                        }
                    } label: {
                        Label(
                            theme.localized("hig.accountSwitcher.addAccount"),
                            systemImage: "person.badge.plus"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(isMutating)
                    .accessibilityLabel(theme.localized("hig.accountSwitcher.addAccount"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.addAccountButton)

                    if coordinator.authState != .unauthenticated {
                        Button(role: .destructive) {
                            Task { await signOutActiveAccount() }
                        } label: {
                            Label(
                                theme.localized("hig.accountSwitcher.signOutActive"),
                                systemImage: "rectangle.portrait.and.arrow.right"
                            )
                            .fabushiFont(.body)
                            .foregroundStyle(theme.destructiveColor)
                            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(isMutating)
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(theme.surfacePrimary)
            .navigationTitle(theme.localized("hig.accountSwitcher.title"))
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        onDismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .fabushiFont(.body)
                            .foregroundStyle(theme.textPrimary)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(theme.localized("hig.common.close"))
                    .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.accountSwitcherCloseButton)
                }
            }
            .navigationDestination(isPresented: $showingAddAccount) {
                FabushiAuthWizardView(
                    coordinator: coordinator,
                    router: router,
                    launchConfiguration: .current,
                    mode: .addAccount,
                    onCancelAddAccount: {
                        showingAddAccount = false
                    },
                    onAuthenticated: {
                        showingAddAccount = false
                        onDismiss()
                    }
                )
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(FabushiAccessibilityIDs.Shell.accountSwitcherSheet)
        .task {
            await loadAccountsAndQuota()
        }
    }

    private func accountRow(account: FabushiAccountSummary, index: Int) -> some View {
        let displayTitle = account.firstName?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
            ? (account.firstName ?? account.accountId)
            : (account.username ?? account.accountId)
        let subtitle = account.username.map { "@\($0)" } ?? account.accountId
        let peerId = Int64(bitPattern: account.userId ?? UInt64(index + 1))

        return Button {
            Task { await switchToAccount(account) }
        } label: {
            HStack(spacing: 12) {
                FabushiAvatarBadge(
                    peerId: peerId,
                    title: displayTitle,
                    isOnline: account.active,
                    isBot: false
                )

                VStack(alignment: .leading, spacing: 2) {
                    Text(displayTitle)
                        .fabushiFont(.body)
                        .foregroundStyle(theme.textPrimary)
                    Text(subtitle)
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                }

                Spacer(minLength: 8)

                if account.active {
                    Image(systemName: "checkmark.circle.fill")
                        .fabushiFont(.title3)
                        .foregroundStyle(theme.accentColor)
                        .accessibilityHidden(true)
                }
            }
            .frame(maxWidth: .infinity, minHeight: FabushiLayoutMetrics.minTouchTarget, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(isMutating)
        .accessibilityLabel(
            account.active
                ? "\(displayTitle), \(theme.localized("hig.accountSwitcher.activeBadge"))"
                : displayTitle
        )
        .accessibilityIdentifier("fabushi.shell.accountSwitcher.row.\(index)")
    }

    private func loadAccountsAndQuota() async {
        guard coordinator.isStarted else { return }
        isLoading = true
        defer { isLoading = false }

        if let listResult = try? await coordinator.host.listAccounts(
            generation: coordinator.accountScopeGeneration
        ) {
            accounts = listResult.accounts
        }

        if let membership: MembershipResponse = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.wallet.method("checkMembership"),
            params: FabushiEmptyParams()
        ) {
            isPremiumAccount = membership.isPremium
        } else if let privacy: PrivacyPremiumResponse = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("privacyGet"),
            params: FabushiEmptyParams()
        ) {
            isPremiumAccount = privacy.isPremium
        }
    }

    private func switchToAccount(_ account: FabushiAccountSummary) async {
        errorMessage = nil
        if account.active {
            onDismiss()
            return
        }
        isMutating = true
        defer { isMutating = false }

        do {
            let newGen = try await coordinator.switchAccount(accountId: account.accountId)
            router.handleAccountScopeChange(generation: newGen, coordinator: coordinator)
            onDismiss()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func signOutActiveAccount() async {
        errorMessage = nil
        isMutating = true
        defer { isMutating = false }

        do {
            let newGen = try await coordinator.logout()
            router.handleAccountScopeChange(generation: newGen, coordinator: coordinator)
            onDismiss()
        } catch {
            if coordinator.authState == .unauthenticated {
                router.handleAccountScopeChange(
                    generation: coordinator.accountScopeGeneration,
                    coordinator: coordinator
                )
                onDismiss()
            } else {
                errorMessage = error.localizedDescription
            }
        }
    }
}
