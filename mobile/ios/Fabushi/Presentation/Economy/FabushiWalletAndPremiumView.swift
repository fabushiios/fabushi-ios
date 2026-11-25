import SwiftUI

/// Unified Apple HIG Wallet, Stars Ledger, Star Gifts, Redeem Codes, Fabushi Premium & StoreKit 2
/// surface returned by `PlatformModulesProviding.makeWalletAndPremiumViews` and embedded in Settings.
@MainActor
public struct FabushiWalletAndPremiumView: View {
    @State private var viewModel: FabushiEconomyViewModel

    @State private var isGiftSheetPresented: Bool = false
    @State private var isRedeemSheetPresented: Bool = false
    @State private var giftRecipientInput: String = ""
    @State private var giftAmountInput: String = "50"
    @State private var giftMessageInput: String = ""
    @State private var redeemCodeInput: String = ""
    @State private var alipayOrderIdInput: String = ""

    @Environment(FabushiTheme.self) private var injectedTheme: FabushiTheme?
    private var theme: FabushiTheme { injectedTheme ?? .default }
    private var locale: FabushiLocale { theme.locale }

    public init(coordinator: FabushiCoordinator, router: ShellNavigationRouter) {
        _viewModel = State(
            initialValue: FabushiEconomyViewModel(
                coordinator: coordinator,
                router: router
            )
        )
    }

    public var body: some View {
        List {
            // MARK: Stars Balance Hero Section
            Section {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 10) {
                        Image(systemName: "star.circle.fill")
                            .fabushiFont(.largeTitle)
                            .foregroundStyle(theme.warningColor)
                            .accessibilityHidden(true)

                        VStack(alignment: .leading, spacing: 2) {
                            Text(FabushiPlatformStrings.text(.economyStarsBalanceTitle, locale: locale))
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                            Text(FabushiPlatformStrings.starsBalanceFormatted(viewModel.balance.starsBalance, locale: locale))
                                .fabushiFont(.title2)
                                .foregroundStyle(theme.textPrimary)
                                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.starsBalanceLabel)
                        }

                        Spacer()

                        Text(
                            viewModel.membership.isPremium
                                ? FabushiPlatformStrings.text(.economyPremiumActiveBadge, locale: locale)
                                : FabushiPlatformStrings.text(.economyPremiumFreeBadge, locale: locale)
                        )
                        .fabushiFont(.caption2)
                        .foregroundStyle(viewModel.membership.isPremium ? theme.successColor : theme.textSecondary)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(theme.surfaceElevated, in: Capsule())
                    }

                    HStack(spacing: 8) {
                        Button {
                            Task { _ = await viewModel.topupStars(amount: 100, locale: locale) }
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.economyTopupStarsAction, locale: locale),
                                systemImage: "plus.circle.fill"
                            )
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.accentForegroundColor)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .background(theme.accentColor, in: Capsule())
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)

                        Button {
                            isGiftSheetPresented = true
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.economyGiftStarsAction, locale: locale),
                                systemImage: "gift.fill"
                            )
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textPrimary)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .background(theme.surfaceElevated, in: Capsule())
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)

                        Button {
                            isRedeemSheetPresented = true
                        } label: {
                            Label(
                                FabushiPlatformStrings.text(.economyRedeemCodeAction, locale: locale),
                                systemImage: "ticket.fill"
                            )
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.textPrimary)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .background(theme.surfaceElevated, in: Capsule())
                            .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 6)
            }

            // MARK: StoreKit 2 Premium & Restore Purchases + Honest BLK-04 Disclosure
            Section(FabushiPlatformStrings.text(.economyPremiumTitle, locale: locale)) {
                ForEach(FabushiStoreKitPlanOption.canonicalCatalog) { option in
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(locale == .zhHans ? option.titleZhHans : option.titleEn)
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.textPrimary)
                            Text(locale == .zhHans ? option.subtitleZhHans : option.subtitleEn)
                                .fabushiFont(.caption)
                                .foregroundStyle(theme.textSecondary)
                        }

                        Spacer()

                        Button {
                            Task { await viewModel.purchaseViaStoreKit(option: option, locale: locale) }
                        } label: {
                            Text(viewModel.storeKitDisplayPrices[option.productId] ?? option.fallbackPriceLabel)
                                .fabushiFont(.subheadline)
                                .foregroundStyle(theme.accentForegroundColor)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 7)
                                .background(theme.accentColor, in: Capsule())
                                .fabushiMinimumTouchTarget()
                        }
                        .buttonStyle(.plain)
                        .disabled(viewModel.isPurchasing)
                        .accessibilityLabel(FabushiPlatformStrings.text(.economyPremiumPurchaseAction, locale: locale))
                    }
                }

                Button {
                    Task { await viewModel.restorePurchases(locale: locale) }
                } label: {
                    HStack {
                        Label(
                            FabushiPlatformStrings.text(.economyStoreKitRestoreAction, locale: locale),
                            systemImage: "arrow.clockwise.circle"
                        )
                        .fabushiFont(.body)
                        .foregroundStyle(theme.accentColor)
                        Spacer()
                        if viewModel.isRestoringPurchases {
                            ProgressView()
                        }
                    }
                    .fabushiMinimumTouchTarget()
                }
                .buttonStyle(.plain)
                .disabled(viewModel.isRestoringPurchases)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.storeKitRestoreButton)

                if let status = viewModel.lastStoreKitStatusMessage, !status.isEmpty {
                    Text(status)
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }

                // ALWAYS surface honest BLK-04 StoreKit 2 server receipt binding disclosure!
                VStack(alignment: .leading, spacing: 4) {
                    Label(
                        FabushiPlatformStrings.text(.economyStoreKitDisclosureTitle, locale: locale),
                        systemImage: "info.circle.fill"
                    )
                    .fabushiFont(.subheadline)
                    .foregroundStyle(theme.warningColor)

                    Text(FabushiPlatformStrings.text(.economyStoreKitDisclosureBody, locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
                .padding(.vertical, 4)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier(FabushiAccessibilityIDs.Settings.storeKitDisclosureBanner)
            }

            // MARK: Honest BLK-03 TON On-Chain Signing Disclosure
            Section(FabushiPlatformStrings.text(.economyTonDisclosureTitle, locale: locale)) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(viewModel.tonStatus.onChainSigning)
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.warningColor)
                        Spacer()
                        Text(viewModel.tonStatus.blockerId)
                            .fabushiFont(.monospacedCaption)
                            .foregroundStyle(theme.textSecondary)
                    }
                    Text(FabushiPlatformStrings.text(.economyTonDisclosureBody, locale: locale))
                        .fabushiFont(.caption)
                        .foregroundStyle(theme.textSecondary)
                }
                .padding(.vertical, 4)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("fabushi.economy.tonDisclosureBanner")
            }

            // MARK: Stars Transaction Ledger (`wallet.transactions`)
            Section(FabushiPlatformStrings.text(.economyTransactionsTitle, locale: locale)) {
                if viewModel.transactions.isEmpty {
                    Text(FabushiPlatformStrings.text(.economyTransactionsEmpty, locale: locale))
                        .fabushiFont(.footnote)
                        .foregroundStyle(theme.textSecondary)
                } else {
                    ForEach(viewModel.transactions) { tx in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(tx.description.isEmpty ? tx.kind : tx.description)
                                    .fabushiFont(.subheadline)
                                    .foregroundStyle(theme.textPrimary)
                                Text(tx.kind)
                                    .fabushiFont(.caption2)
                                    .foregroundStyle(theme.textSecondary)
                            }
                            Spacer()
                            Text(tx.amountStars >= 0 ? "+\(tx.amountStars) ★" : "\(tx.amountStars) ★")
                                .fabushiFont(.monospacedCaption)
                                .foregroundStyle(tx.amountStars >= 0 ? theme.successColor : theme.destructiveColor)
                        }
                    }
                }
            }

            // MARK: Order Lookup (`wallet.queryAlipayOrder`)
            Section(FabushiPlatformStrings.text(.economyAlipaySectionTitle, locale: locale)) {
                HStack(spacing: 8) {
                    TextField(
                        FabushiPlatformStrings.text(.economyAlipayOrderIdPlaceholder, locale: locale),
                        text: $alipayOrderIdInput
                    )
                    .fabushiFont(.body)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()

                    Button {
                        Task { _ = await viewModel.queryAlipayOrder(orderId: alipayOrderIdInput) }
                    } label: {
                        Text(FabushiPlatformStrings.text(.economyAlipayQueryOrder, locale: locale))
                            .fabushiFont(.footnote)
                            .foregroundStyle(theme.accentColor)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }

                if let query = viewModel.lastOrderQuery {
                    Text("\(query.orderId) · \(query.status) · \(query.amount)")
                        .fabushiFont(.monospacedCaption)
                        .foregroundStyle(theme.textSecondary)
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(theme.surfacePrimary)
        .navigationTitle(FabushiPlatformStrings.text(.economyTitle, locale: locale))
        .refreshable {
            await viewModel.refreshAll()
        }
        .task {
            await viewModel.refreshAll()
        }
        .onDisappear {
            viewModel.detachLifecycleHooks()
        }
        .sheet(isPresented: $isGiftSheetPresented) {
            starGiftSheet
        }
        .sheet(isPresented: $isRedeemSheetPresented) {
            redeemCodeSheet
        }
    }

    private var starGiftSheet: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiPlatformStrings.text(.economyGiftRecipientPlaceholder, locale: locale),
                        text: $giftRecipientInput
                    )
                    .keyboardType(.numberPad)
                    .fabushiFont(.body)

                    TextField(
                        FabushiPlatformStrings.text(.economyGiftAmountPlaceholder, locale: locale),
                        text: $giftAmountInput
                    )
                    .keyboardType(.numberPad)
                    .fabushiFont(.body)

                    TextField(
                        FabushiPlatformStrings.text(.economyGiftMessagePlaceholder, locale: locale),
                        text: $giftMessageInput
                    )
                    .fabushiFont(.body)
                }

                Section {
                    Button {
                        let recipient = UInt64(giftRecipientInput.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
                        let amount = Int64(giftAmountInput.trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
                        Task {
                            let ok = await viewModel.giftStars(
                                toUserId: recipient,
                                amountStars: amount,
                                message: giftMessageInput,
                                locale: locale
                            )
                            if ok {
                                isGiftSheetPresented = false
                                giftMessageInput = ""
                            }
                        }
                    } label: {
                        Text(FabushiPlatformStrings.text(.economyGiftSendAction, locale: locale))
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.accentColor)
                            .frame(maxWidth: .infinity)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }
            .navigationTitle(FabushiPlatformStrings.text(.economyGiftStarsAction, locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.cancel, locale: locale)) {
                        isGiftSheetPresented = false
                    }
                }
            }
        }
        .presentationDetents([.medium])
    }

    private var redeemCodeSheet: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        FabushiPlatformStrings.text(.economyRedeemCodePlaceholder, locale: locale),
                        text: $redeemCodeInput
                    )
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                    .fabushiFont(.body)
                }

                Section {
                    Button {
                        Task {
                            let res = await viewModel.redeemCode(redeemCodeInput, locale: locale)
                            if res?.success == true {
                                isRedeemSheetPresented = false
                                redeemCodeInput = ""
                            }
                        }
                    } label: {
                        Text(FabushiPlatformStrings.text(.economyRedeemCodeSubmit, locale: locale))
                            .fabushiFont(.headline)
                            .foregroundStyle(theme.accentColor)
                            .frame(maxWidth: .infinity)
                            .fabushiMinimumTouchTarget()
                    }
                    .buttonStyle(.plain)
                }
            }
            .navigationTitle(FabushiPlatformStrings.text(.economyRedeemCodeAction, locale: locale))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(FabushiPlatformStrings.text(.cancel, locale: locale)) {
                        isRedeemSheetPresented = false
                    }
                }
            }
        }
        .presentationDetents([.medium])
    }
}
