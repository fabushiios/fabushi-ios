import Foundation

// MARK: - Economy, Stars, Gifts, Premium & TON DTOs (Feature #31)

public struct FabushiWalletBalanceDto: Codable, Sendable, Equatable {
    public let userId: UInt64
    public let starsBalance: Int64
    public let tonNano: Int64
    public let updatedAtMs: Int64

    enum CodingKeys: String, CodingKey {
        case userId = "user_id"
        case starsBalance = "stars_balance"
        case tonNano = "ton_nano"
        case updatedAtMs = "updated_at_ms"
    }

    public init(
        userId: UInt64 = 0,
        starsBalance: Int64 = 0,
        tonNano: Int64 = 0,
        updatedAtMs: Int64 = 0
    ) {
        self.userId = userId
        self.starsBalance = starsBalance
        self.tonNano = tonNano
        self.updatedAtMs = updatedAtMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        userId = try c.decodeIfPresent(UInt64.self, forKey: .userId) ?? 0
        starsBalance = try c.decodeIfPresent(Int64.self, forKey: .starsBalance) ?? 0
        tonNano = try c.decodeIfPresent(Int64.self, forKey: .tonNano) ?? 0
        updatedAtMs = try c.decodeIfPresent(Int64.self, forKey: .updatedAtMs) ?? 0
    }
}

public struct FabushiWalletTransactionDto: Codable, Identifiable, Sendable, Equatable {
    public let txId: String
    public let kind: String
    public let amountStars: Int64
    public let peerUserId: UInt64?
    public let description: String
    public let createdAtMs: Int64

    public var id: String { txId }

    enum CodingKeys: String, CodingKey {
        case txId = "tx_id"
        case kind
        case amountStars = "amount_stars"
        case peerUserId = "peer_user_id"
        case description
        case createdAtMs = "created_at_ms"
    }

    public init(
        txId: String,
        kind: String,
        amountStars: Int64,
        peerUserId: UInt64? = nil,
        description: String = "",
        createdAtMs: Int64 = 0
    ) {
        self.txId = txId
        self.kind = kind
        self.amountStars = amountStars
        self.peerUserId = peerUserId
        self.description = description
        self.createdAtMs = createdAtMs
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        txId = try c.decode(String.self, forKey: .txId)
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "transfer"
        amountStars = try c.decodeIfPresent(Int64.self, forKey: .amountStars) ?? 0
        peerUserId = try c.decodeIfPresent(UInt64.self, forKey: .peerUserId)
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        createdAtMs = try c.decodeIfPresent(Int64.self, forKey: .createdAtMs) ?? 0
    }
}

public struct FabushiStarGiftRequest: Encodable, Sendable {
    public let toUserId: UInt64
    public let chatId: Int64?
    public let amountStars: Int64
    public let giftId: String?
    public let message: String?

    enum CodingKeys: String, CodingKey {
        case toUserId = "to_user_id"
        case chatId = "chat_id"
        case amountStars = "amount_stars"
        case giftId = "gift_id"
        case message
    }

    public init(
        toUserId: UInt64,
        chatId: Int64? = nil,
        amountStars: Int64,
        giftId: String? = nil,
        message: String? = nil
    ) {
        self.toUserId = toUserId
        self.chatId = chatId
        self.amountStars = amountStars
        self.giftId = giftId
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.message = (trimmed?.isEmpty == false) ? trimmed : nil
    }
}

public struct FabushiRedeemCodeResponse: Decodable, Sendable, Equatable {
    public let success: Bool
    public let plan: String
    public let durationDays: UInt32
    public let expiresAt: Int64
    public let message: String?

    enum CodingKeys: String, CodingKey {
        case success
        case plan
        case durationDays = "duration_days"
        case expiresAt = "expires_at"
        case message
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        success = try c.decodeIfPresent(Bool.self, forKey: .success) ?? false
        plan = try c.decodeIfPresent(String.self, forKey: .plan) ?? ""
        durationDays = try c.decodeIfPresent(UInt32.self, forKey: .durationDays) ?? 0
        expiresAt = try c.decodeIfPresent(Int64.self, forKey: .expiresAt) ?? 0
        message = try c.decodeIfPresent(String.self, forKey: .message)
    }
}

public struct FabushiMembershipStatusDto: Decodable, Sendable, Equatable {
    public let success: Bool
    public let isPremium: Bool
    public let plan: String
    public let daysLeft: UInt32
    public let expiresAtMs: Int64?
    public let username: String

    enum CodingKeys: String, CodingKey {
        case success
        case isPremium = "is_premium"
        case plan
        case daysLeft = "days_left"
        case expiresAtMs = "expires_at_ms"
        case username
    }

    public init(
        success: Bool = false,
        isPremium: Bool = false,
        plan: String = "free",
        daysLeft: UInt32 = 0,
        expiresAtMs: Int64? = nil,
        username: String = ""
    ) {
        self.success = success
        self.isPremium = isPremium
        self.plan = plan
        self.daysLeft = daysLeft
        self.expiresAtMs = expiresAtMs
        self.username = username
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        success = try c.decodeIfPresent(Bool.self, forKey: .success) ?? false
        isPremium = try c.decodeIfPresent(Bool.self, forKey: .isPremium) ?? false
        plan = try c.decodeIfPresent(String.self, forKey: .plan) ?? "free"
        daysLeft = try c.decodeIfPresent(UInt32.self, forKey: .daysLeft) ?? 0
        expiresAtMs = try c.decodeIfPresent(Int64.self, forKey: .expiresAtMs)
        username = try c.decodeIfPresent(String.self, forKey: .username) ?? ""
    }
}

public struct FabushiAlipayOrderResponse: Decodable, Sendable, Equatable {
    public let success: Bool
    public let orderId: String
    public let amount: String
    public let plan: String
    public let productType: String?
    public let paymentUrl: String?

    enum CodingKeys: String, CodingKey {
        case success
        case orderId = "order_id"
        case amount
        case plan
        case productType = "product_type"
        case paymentUrl = "payment_url"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        success = try c.decodeIfPresent(Bool.self, forKey: .success) ?? false
        orderId = try c.decodeIfPresent(String.self, forKey: .orderId) ?? ""
        amount = try c.decodeIfPresent(String.self, forKey: .amount) ?? ""
        plan = try c.decodeIfPresent(String.self, forKey: .plan) ?? ""
        productType = try c.decodeIfPresent(String.self, forKey: .productType)
        paymentUrl = try c.decodeIfPresent(String.self, forKey: .paymentUrl)
    }
}

public struct FabushiAlipayOrderQueryResponse: Decodable, Sendable, Equatable {
    public let success: Bool
    public let orderId: String
    public let plan: String
    public let amount: String
    public let status: String
    public let createdAt: String?

    enum CodingKeys: String, CodingKey {
        case success
        case orderId = "order_id"
        case plan
        case amount
        case status
        case createdAt = "created_at"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        success = try c.decodeIfPresent(Bool.self, forKey: .success) ?? false
        orderId = try c.decodeIfPresent(String.self, forKey: .orderId) ?? ""
        plan = try c.decodeIfPresent(String.self, forKey: .plan) ?? ""
        amount = try c.decodeIfPresent(String.self, forKey: .amount) ?? ""
        status = try c.decodeIfPresent(String.self, forKey: .status) ?? "unknown"
        createdAt = try c.decodeIfPresent(String.self, forKey: .createdAt)
    }
}

public struct FabushiTonStatusDto: Decodable, Sendable, Equatable {
    public let onChainSigning: String
    public let blockerId: String
    public let storekit2Runner: String

    enum CodingKeys: String, CodingKey {
        case onChainSigning = "on_chain_signing"
        case blockerId = "blocker_id"
        case storekit2Runner = "storekit2_runner"
    }

    public init(
        onChainSigning: String = "upstream_not_implemented",
        blockerId: String = "BLK-03",
        storekit2Runner: String = "native_ios_swift"
    ) {
        self.onChainSigning = onChainSigning
        self.blockerId = blockerId
        self.storekit2Runner = storekit2Runner
    }
}

public struct FabushiStoreKitPlanOption: Identifiable, Sendable, Equatable {
    public let id: String
    public let productId: String
    public let sku: String
    public let titleEn: String
    public let titleZhHans: String
    public let subtitleEn: String
    public let subtitleZhHans: String
    public let fallbackPriceLabel: String

    public init(
        id: String,
        productId: String,
        sku: String,
        titleEn: String,
        titleZhHans: String,
        subtitleEn: String,
        subtitleZhHans: String,
        fallbackPriceLabel: String
    ) {
        self.id = id
        self.productId = productId
        self.sku = sku
        self.titleEn = titleEn
        self.titleZhHans = titleZhHans
        self.subtitleEn = subtitleEn
        self.subtitleZhHans = subtitleZhHans
        self.fallbackPriceLabel = fallbackPriceLabel
    }

    public static let canonicalCatalog: [FabushiStoreKitPlanOption] = [
        FabushiStoreKitPlanOption(
            id: "premium-monthly",
            productId: GlobalDharmaCommerceModel.monthlyProductId,
            sku: GlobalDharmaCommerceModel.monthlySku,
            titleEn: "Fabushi Premium · Monthly",
            titleZhHans: "Fabushi 会员 · 按月订阅",
            subtitleEn: "App Store StoreKit 2 auto-renewable plan",
            subtitleZhHans: "通过 App Store StoreKit 2 订阅",
            fallbackPriceLabel: "¥30 / mo"
        ),
        FabushiStoreKitPlanOption(
            id: "premium-lifetime",
            productId: GlobalDharmaCommerceModel.lifetimeProductId,
            sku: GlobalDharmaCommerceModel.lifetimeSku,
            titleEn: "Global Dharma · Lifetime",
            titleZhHans: "全球法布施 · 永久买断",
            subtitleEn: "App Store StoreKit 2 non-consumable purchase",
            subtitleZhHans: "通过 App Store StoreKit 2 非消耗型购买",
            fallbackPriceLabel: "¥1080"
        ),
    ]
}
