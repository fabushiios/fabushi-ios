import Foundation
import Observation
import os
import StoreKit

/// Generation-fenced ViewModel driving Economy, Stars Ledger, Star Gifts, Redeem Codes,
/// Fabushi Premium, App Store Guideline 3.1.1 StoreKit 2 (`FabushiPayStoreKit`),
/// and honest `BLK-04` (StoreKit 2 server binding) + `BLK-03` (TON on-chain signing) disclosures.
@MainActor
@Observable
public final class FabushiEconomyViewModel {
    public let coordinator: FabushiCoordinator
    public let router: ShellNavigationRouter

    public private(set) var balance: FabushiWalletBalanceDto = FabushiWalletBalanceDto()
    public private(set) var transactions: [FabushiWalletTransactionDto] = []
    public private(set) var membership: FabushiMembershipStatusDto = FabushiMembershipStatusDto()
    public private(set) var tonStatus: FabushiTonStatusDto = FabushiTonStatusDto()
    public private(set) var lastOrder: FabushiAlipayOrderResponse? = nil
    public private(set) var lastOrderQuery: FabushiAlipayOrderQueryResponse? = nil
    public private(set) var lastRedeemResult: FabushiRedeemCodeResponse? = nil
    public private(set) var storeKitDisplayPrices: [String: String] = [:]
    public private(set) var localVerifiedStoreKitProductIds: [String] = []
    public private(set) var lastStoreKitStatusMessage: String? = nil

    /// Honest BLK-04 disclosure (server-side StoreKit 2 JWS receipt/entitlement binding unprovisioned).
    public let isBlk04StoreKitDisclosureVisible: Bool = true
    /// Honest BLK-03 disclosure (TON on-chain signing & broadcast are upstream_not_implemented).
    public let isBlk03TonDisclosureVisible: Bool = true

    public private(set) var isLoading: Bool = false
    public private(set) var isRestoringPurchases: Bool = false
    public private(set) var isPurchasing: Bool = false
    public private(set) var lastErrorMessage: String? = nil
    public private(set) var observedGeneration: UInt64

    @ObservationIgnored private let payStoreKit: FabushiPayStoreKit
    @ObservationIgnored private let purgerId = "m5.economy.\(UUID().uuidString)"

    public init(
        coordinator: FabushiCoordinator,
        router: ShellNavigationRouter,
        payServiceBaseURL: URL = URL(string: "https://pay.ombhrum.com")!
    ) {
        self.coordinator = coordinator
        self.router = router
        self.observedGeneration = coordinator.accountScopeGeneration
        self.payStoreKit = FabushiPayStoreKit(
            serviceBaseURL: payServiceBaseURL,
            session: .shared,
            accessTokenProvider: {
                guard case .authenticated(_, _, let accountId) = coordinator.authState,
                      !accountId.isEmpty
                else {
                    throw FabushiPayStoreKitError.invalidServerResponse
                }
                return accountId
            }
        )
        installAccountResetHooks()
    }

    // MARK: - Lifecycle & Account-Switch Wipe

    private func installAccountResetHooks() {
        coordinator.registerAccountSecretsPurger(id: purgerId) { [weak self] _ in
            self?.resetStateForAccountScopeChange()
        }
        coordinator.registerServerSwitchSecretsPurger(id: purgerId) { [weak self] in
            self?.resetStateForAccountScopeChange()
        }
    }

    public func detachLifecycleHooks() {
        coordinator.unregisterAccountSecretsPurger(id: purgerId)
        coordinator.unregisterServerSwitchSecretsPurger(id: purgerId)
    }

    public func reconcileAccountScopeIfNeeded() {
        let current = coordinator.accountScopeGeneration
        guard current != observedGeneration else { return }
        resetStateForAccountScopeChange()
    }

    private func resetStateForAccountScopeChange() {
        observedGeneration = coordinator.accountScopeGeneration
        balance = FabushiWalletBalanceDto()
        transactions = []
        membership = FabushiMembershipStatusDto()
        tonStatus = FabushiTonStatusDto()
        lastOrder = nil
        lastOrderQuery = nil
        lastRedeemResult = nil
        localVerifiedStoreKitProductIds = []
        lastStoreKitStatusMessage = nil
        lastErrorMessage = nil
        isLoading = false
        isRestoringPurchases = false
        isPurchasing = false
    }

    // MARK: - Full Refresh (`wallet.balance`, `wallet.transactions`, `wallet.checkMembership`, `wallet.tonStatus`)

    public func refreshAll() async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isLoading = true
        lastErrorMessage = nil
        defer { isLoading = false }

        // Always query real local Rust `wallet.tonStatus` (succeeds offline & confirms BLK-03).
        if let ton: FabushiTonStatusDto = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.wallet.method("tonStatus"),
            params: FabushiEmptyParams()
        ) {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            tonStatus = ton
        }

        do {
            let fetchedBalance: FabushiWalletBalanceDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("balance"),
                params: FabushiEmptyParams()
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            balance = fetchedBalance
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastErrorMessage = error.localizedDescription
        }

        if let txs: [FabushiWalletTransactionDto] = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.wallet.method("transactions"),
            params: FabushiEmptyParams()
        ) {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            transactions = txs
        }

        if let member: FabushiMembershipStatusDto = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.wallet.method("checkMembership"),
            params: FabushiEmptyParams()
        ) {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            membership = member
        }

        await refreshStoreKitCatalog()
    }

    // MARK: - Stars Top-Up, Gifts & Redeem Codes (`wallet.topup`, `wallet.giftStars`, `wallet.redeemCode`)

    @discardableResult
    public func topupStars(amount: Int64, locale: FabushiLocale = .en) async -> Bool {
        reconcileAccountScopeIfNeeded()
        guard amount > 0 else { return false }
        let capturedGen = coordinator.accountScopeGeneration

        struct TopupParams: Encodable, Sendable {
            let amount_stars: Int64
        }

        do {
            let updated: FabushiWalletBalanceDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("topup"),
                params: TopupParams(amount_stars: amount)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            balance = updated
            lastErrorMessage = nil
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.starsBalanceFormatted(updated.starsBalance, locale: locale),
                isError: false
            )
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    @discardableResult
    public func giftStars(
        toUserId: UInt64,
        amountStars: Int64,
        chatId: Int64? = nil,
        message: String? = nil,
        locale: FabushiLocale = .en
    ) async -> Bool {
        reconcileAccountScopeIfNeeded()
        guard toUserId > 0, amountStars > 0 else { return false }
        let capturedGen = coordinator.accountScopeGeneration

        do {
            let updated: FabushiWalletBalanceDto = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("giftStars"),
                params: FabushiStarGiftRequest(
                    toUserId: toUserId,
                    chatId: chatId,
                    amountStars: amountStars,
                    message: message
                )
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            balance = updated
            lastErrorMessage = nil
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.economyGiftStarsAction, locale: locale),
                isError: false
            )
            return true
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return false }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return false
        }
    }

    @discardableResult
    public func redeemCode(_ rawCode: String, locale: FabushiLocale = .en) async -> FabushiRedeemCodeResponse? {
        reconcileAccountScopeIfNeeded()
        let code = rawCode.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !code.isEmpty else { return nil }
        let capturedGen = coordinator.accountScopeGeneration

        struct RedeemParams: Encodable, Sendable {
            let code: String
        }

        do {
            let result: FabushiRedeemCodeResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("redeemCode"),
                params: RedeemParams(code: code)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastRedeemResult = result
            lastErrorMessage = nil
            if result.success {
                if let member: FabushiMembershipStatusDto = try? await coordinator.dispatchFenced(
                    method: FabushiRPCNamespace.wallet.method("checkMembership"),
                    params: FabushiEmptyParams()
                ), coordinator.accountScopeGeneration == capturedGen {
                    membership = member
                }
                router.toastController.showNonChatToast(
                    result.message ?? result.plan,
                    isError: false
                )
            }
            return result
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    // MARK: - Alipay Order Status (`wallet.createAlipayOrder`, `wallet.queryAlipayOrder`)

    @discardableResult
    public func createAlipayOrder(plan: String) async -> FabushiAlipayOrderResponse? {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration

        struct CreateOrderParams: Encodable, Sendable {
            let plan: String
            let platform: String
        }

        do {
            let res: FabushiAlipayOrderResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("createAlipayOrder"),
                params: CreateOrderParams(plan: plan, platform: "ios")
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastOrder = res
            return res
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
            return nil
        }
    }

    @discardableResult
    public func queryAlipayOrder(orderId: String) async -> FabushiAlipayOrderQueryResponse? {
        reconcileAccountScopeIfNeeded()
        let trimmed = orderId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let capturedGen = coordinator.accountScopeGeneration

        struct QueryOrderParams: Encodable, Sendable {
            let order_id: String
        }

        do {
            let res: FabushiAlipayOrderQueryResponse = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("queryAlipayOrder"),
                params: QueryOrderParams(order_id: trimmed)
            )
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastOrderQuery = res
            return res
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return nil }
            lastErrorMessage = error.localizedDescription
            return nil
        }
    }

    // MARK: - StoreKit 2 (App Store Guideline 3.1.1 via `FabushiPayStoreKit` + `AppStore.sync()`)

    public func refreshStoreKitCatalog() async {
        let productIds = FabushiStoreKitPlanOption.canonicalCatalog.map(\.productId)
        let fetchedPrices: [String: String]? = await withCheckedContinuation { continuation in
            let settled = OSAllocatedUnfairLock(initialState: false)
            let finish: @Sendable ([String: String]?) -> Void = { value in
                let shouldResume = settled.withLock { done -> Bool in
                    if done { return false }
                    done = true
                    return true
                }
                if shouldResume {
                    continuation.resume(returning: value)
                }
            }
            Task {
                guard let products = try? await Product.products(for: productIds) else {
                    finish(nil)
                    return
                }
                var map: [String: String] = [:]
                for product in products {
                    map[product.id] = product.displayPrice
                }
                finish(map)
            }
            Task {
                try? await Task.sleep(for: .seconds(2))
                finish(nil)
            }
        }
        if let fetchedPrices {
            storeKitDisplayPrices = fetchedPrices
        }
    }

    public func purchaseViaStoreKit(
        option: FabushiStoreKitPlanOption,
        compactJWS: String? = nil,
        locale: FabushiLocale = .en
    ) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isPurchasing = true
        lastErrorMessage = nil
        defer { isPurchasing = false }

        let paymentId = UUID().uuidString.lowercased()
        let verifyPath = "/v1/pay/intents/\(paymentId)/apple/verify"

        do {
            let receipt: FabushiPayReceipt
            if let jws = compactJWS, !jws.isEmpty {
                if #available(iOS 18.4, *) {
                    receipt = try await payStoreKit.purchaseAdvancedCommerce(
                        paymentId: paymentId,
                        genericProductId: option.productId,
                        compactJWS: jws,
                        verifyPath: verifyPath
                    )
                } else {
                    throw FabushiPayStoreKitError.advancedCommerceUnavailable
                }
            } else {
                receipt = try await payStoreKit.purchase(
                    paymentId: paymentId,
                    productId: option.productId,
                    verifyPath: verifyPath
                )
            }
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastStoreKitStatusMessage = "\(receipt.sku): \(receipt.status)"
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.economyPremiumActiveBadge, locale: locale),
                isError: false
            )
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            // Honest BLK-04 feedback when App Store product or server JWS verification is unprovisioned.
            lastStoreKitStatusMessage = error.localizedDescription
            lastErrorMessage = error.localizedDescription
            router.toastController.showNonChatToast(error.localizedDescription, isError: true)
        }
    }

    public func restorePurchases(locale: FabushiLocale = .en) async {
        reconcileAccountScopeIfNeeded()
        let capturedGen = coordinator.accountScopeGeneration
        isRestoringPurchases = true
        lastErrorMessage = nil
        defer { isRestoringPurchases = false }

        let expectedSkus = Set(FabushiStoreKitPlanOption.canonicalCatalog.map(\.sku))
        do {
            let receipt = try await payStoreKit.restore(expectedSkus: expectedSkus)
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            lastStoreKitStatusMessage = "\(receipt.sku): \(receipt.status)"
            router.toastController.showNonChatToast(
                FabushiPlatformStrings.text(.economyPremiumActiveBadge, locale: locale),
                isError: false
            )
            return
        } catch {
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            // Also inspect local StoreKit 2 current entitlements directly so client-side App Store
            // sync state is honestly reflected while server JWS binding remains disclosed under BLK-04.
            var verifiedIds: [String] = []
            for await verification in Transaction.currentEntitlements {
                if case .verified(let transaction) = verification {
                    verifiedIds.append(transaction.productID)
                }
            }
            guard coordinator.accountScopeGeneration == capturedGen else { return }
            localVerifiedStoreKitProductIds = verifiedIds
            let summary = error.localizedDescription
            lastStoreKitStatusMessage = summary
            router.toastController.showNonChatToast(summary, isError: true)
        }
    }

    // MARK: - Honest BLK-03 TON Fail-Closed Check

    @discardableResult
    public func probeTonSignBroadcastFailClosed() async -> FabushiHostError? {
        reconcileAccountScopeIfNeeded()
        do {
            let _: FabushiJSONValue = try await coordinator.dispatchFenced(
                method: FabushiRPCNamespace.wallet.method("tonSignBroadcast"),
                params: FabushiEmptyParams()
            )
            return nil
        } catch let hostError as FabushiHostError {
            return hostError
        } catch {
            return nil
        }
    }
}
