import Foundation

// MARK: - Automation App Store REST Transport Contract (`/api/v1/automation-apps*`)

public enum FabushiAutomationStoreTransportError: LocalizedError, Sendable, Equatable {
    case invalidBaseURL(String)
    case invalidAppID(String)
    case httpError(statusCode: Int, message: String)
    case decodingFailed(String)

    public var errorDescription: String? {
        switch self {
        case .invalidBaseURL(let url):
            return "Invalid Automation App Store server URL: \(url)"
        case .invalidAppID(let id):
            return "Invalid automation app_id '\(id)' (must be 1..128 [a-zA-Z0-9_-] and not 'installed')."
        case .httpError(let statusCode, let message):
            return "Automation App Store HTTP \(statusCode): \(message)"
        case .decodingFailed(let detail):
            return "Failed to decode Automation App Store response: \(detail)"
        }
    }
}

public protocol FabushiAutomationAppStoreTransport: Sendable {
    func searchApps(query: FabushiListAutomationAppsQuery) async throws -> FabushiListAutomationAppsResponse
    func getAppDetail(appId: String) async throws -> FabushiAutomationAppDetail
    func publishApp(request: FabushiPublishAutomationAppRequest) async throws -> FabushiAutomationAppDetail
    func installApp(appId: String) async throws -> FabushiInstallAutomationAppResponse
    func uninstallApp(appId: String) async throws -> FabushiUninstallAutomationAppResponse
    func listInstalledApps() async throws -> FabushiListInstalledAutomationAppsResponse
    func recordRun(appId: String, request: FabushiRecordAutomationRunRequest) async throws -> FabushiRecordAutomationRunResponse
}

/// Real `URLSession` client implementing all 7 `/api/v1/automation-apps*` REST operations
/// matching `gpui_telegram_client::net::api_automation_apps` verbatim, cleanly swappable
/// with M6 `automation_apps.*` Rust host RPCs without modifying frozen `source/**`.
public final class FabushiURLSessionAutomationStoreTransport: FabushiAutomationAppStoreTransport, Sendable {
    private let baseURLProvider: @Sendable () async -> String
    private let bearerTokenProvider: @Sendable () async -> String?
    private let session: URLSession

    public init(
        baseURLProvider: @escaping @Sendable () async -> String,
        bearerTokenProvider: @escaping @Sendable () async -> String? = { nil },
        session: URLSession = .shared
    ) {
        self.baseURLProvider = baseURLProvider
        self.bearerTokenProvider = bearerTokenProvider
        self.session = session
    }

    public func searchApps(
        query: FabushiListAutomationAppsQuery
    ) async throws -> FabushiListAutomationAppsResponse {
        var queryItems: [URLQueryItem] = []
        if let q = query.q?.trimmingCharacters(in: .whitespacesAndNewlines), !q.isEmpty {
            queryItems.append(URLQueryItem(name: "q", value: q))
        }
        if let category = query.category?.trimmingCharacters(in: .whitespacesAndNewlines), !category.isEmpty {
            queryItems.append(URLQueryItem(name: "category", value: category))
        }
        if let domain = query.domain?.trimmingCharacters(in: .whitespacesAndNewlines), !domain.isEmpty {
            queryItems.append(URLQueryItem(name: "domain", value: domain))
        }
        if let limit = query.limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        if let offset = query.offset {
            queryItems.append(URLQueryItem(name: "offset", value: String(offset)))
        }
        return try await performRequest(
            method: "GET",
            path: "/api/v1/automation-apps",
            queryItems: queryItems,
            body: Optional<FabushiEmptyParams>.none
        )
    }

    public func getAppDetail(appId: String) async throws -> FabushiAutomationAppDetail {
        guard FabushiBrowserSecurity.validateAutomationAppID(appId) else {
            throw FabushiAutomationStoreTransportError.invalidAppID(appId)
        }
        return try await performRequest(
            method: "GET",
            path: "/api/v1/automation-apps/\(appId)",
            queryItems: [],
            body: Optional<FabushiEmptyParams>.none
        )
    }

    public func publishApp(
        request: FabushiPublishAutomationAppRequest
    ) async throws -> FabushiAutomationAppDetail {
        return try await performRequest(
            method: "POST",
            path: "/api/v1/automation-apps",
            queryItems: [],
            body: request
        )
    }

    public func installApp(appId: String) async throws -> FabushiInstallAutomationAppResponse {
        guard FabushiBrowserSecurity.validateAutomationAppID(appId) else {
            throw FabushiAutomationStoreTransportError.invalidAppID(appId)
        }
        return try await performRequest(
            method: "POST",
            path: "/api/v1/automation-apps/\(appId)/install",
            queryItems: [],
            body: Optional<FabushiEmptyParams>.none
        )
    }

    public func uninstallApp(appId: String) async throws -> FabushiUninstallAutomationAppResponse {
        guard FabushiBrowserSecurity.validateAutomationAppID(appId) else {
            throw FabushiAutomationStoreTransportError.invalidAppID(appId)
        }
        return try await performRequest(
            method: "DELETE",
            path: "/api/v1/automation-apps/\(appId)/install",
            queryItems: [],
            body: Optional<FabushiEmptyParams>.none
        )
    }

    public func listInstalledApps() async throws -> FabushiListInstalledAutomationAppsResponse {
        return try await performRequest(
            method: "GET",
            path: "/api/v1/automation-apps/installed",
            queryItems: [],
            body: Optional<FabushiEmptyParams>.none
        )
    }

    public func recordRun(
        appId: String,
        request: FabushiRecordAutomationRunRequest
    ) async throws -> FabushiRecordAutomationRunResponse {
        guard FabushiBrowserSecurity.validateAutomationAppID(appId) else {
            throw FabushiAutomationStoreTransportError.invalidAppID(appId)
        }
        return try await performRequest(
            method: "POST",
            path: "/api/v1/automation-apps/\(appId)/runs",
            queryItems: [],
            body: request
        )
    }

    private func performRequest<Body: Encodable, Res: Decodable>(
        method: String,
        path: String,
        queryItems: [URLQueryItem],
        body: Body?
    ) async throws -> Res {
        let baseRaw = (await baseURLProvider()).trimmingCharacters(in: .whitespacesAndNewlines)
        guard let baseURL = FabushiBrowserSecurity.validateWKNavigationURL(baseRaw),
              var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)
        else {
            throw FabushiAutomationStoreTransportError.invalidBaseURL(baseRaw)
        }
        let basePath = components.path.hasSuffix("/") ? String(components.path.dropLast()) : components.path
        components.path = basePath + path
        if !queryItems.isEmpty {
            components.queryItems = queryItems
        }
        guard let finalURL = components.url else {
            throw FabushiAutomationStoreTransportError.invalidBaseURL(baseRaw)
        }

        var request = URLRequest(url: finalURL)
        request.httpMethod = method
        request.timeoutInterval = 20
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let token = await bearerTokenProvider(), !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            request.httpBody = try JSONEncoder().encode(body)
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw FabushiAutomationStoreTransportError.httpError(statusCode: 0, message: "Non-HTTP response")
        }
        guard (200..<300).contains(http.statusCode) else {
            let msg = String(decoding: data, as: UTF8.self)
            throw FabushiAutomationStoreTransportError.httpError(
                statusCode: http.statusCode,
                message: msg.isEmpty ? HTTPURLResponse.localizedString(forStatusCode: http.statusCode) : msg
            )
        }
        do {
            return try JSONDecoder().decode(Res.self, from: data)
        } catch {
            throw FabushiAutomationStoreTransportError.decodingFailed(error.localizedDescription)
        }
    }
}

// MARK: - Offline Store Cache Persisted via Real `settings.localSet` / `settings.localGet`

@MainActor
public final class FabushiAutomationAppOfflineCache {
    public static let installedCacheKey = "browser.automation_apps.installed_cache.v1"
    public static let catalogCacheKey = "browser.automation_apps.catalog_cache.v1"

    public let coordinator: FabushiCoordinator

    public init(coordinator: FabushiCoordinator) {
        self.coordinator = coordinator
    }

    public func saveInstalledApps(_ apps: [FabushiInstalledAutomationApp]) async {
        guard let data = try? JSONEncoder().encode(apps),
              let jsonVal = try? JSONDecoder().decode(FabushiJSONValue.self, from: data)
        else { return }
        await setLocalSetting(key: Self.installedCacheKey, value: jsonVal)
    }

    public func loadInstalledApps() async -> [FabushiInstalledAutomationApp] {
        guard let val = await getLocalSetting(key: Self.installedCacheKey),
              val != .null,
              let data = val.toJSONString().data(using: .utf8),
              let decoded = try? JSONDecoder().decode([FabushiInstalledAutomationApp].self, from: data)
        else {
            return []
        }
        return decoded
    }

    public func saveCatalogSummaries(_ apps: [FabushiAutomationAppSummary]) async {
        guard let data = try? JSONEncoder().encode(apps),
              let jsonVal = try? JSONDecoder().decode(FabushiJSONValue.self, from: data)
        else { return }
        await setLocalSetting(key: Self.catalogCacheKey, value: jsonVal)
    }

    public func loadCatalogSummaries() async -> [FabushiAutomationAppSummary] {
        guard let val = await getLocalSetting(key: Self.catalogCacheKey),
              val != .null,
              let data = val.toJSONString().data(using: .utf8),
              let decoded = try? JSONDecoder().decode([FabushiAutomationAppSummary].self, from: data)
        else {
            return []
        }
        return decoded
    }

    /// Wipes both installed and catalog offline caches cleanly (matching desktop `sqlite_cache.rs:L375-L376`
    /// `DELETE FROM installed_automation_apps; DELETE FROM automation_app_cache;`) on logout / account switch.
    public func wipeAll() async {
        await setLocalSetting(key: Self.installedCacheKey, value: .array([]))
        await setLocalSetting(key: Self.catalogCacheKey, value: .array([]))
    }

    private func setLocalSetting(key: String, value: FabushiJSONValue) async {
        struct Params: Encodable, Sendable {
            let key: String
            let value: FabushiJSONValue
        }
        _ = try? await coordinator.host.dispatch(
            method: FabushiRPCNamespace.settings.method("localSet"),
            params: Params(key: key, value: value),
            generation: nil,
            accountId: coordinator.activeAccountId
        ) as FabushiJSONValue
    }

    private func getLocalSetting(key: String) async -> FabushiJSONValue? {
        struct Params: Encodable, Sendable { let key: String }
        struct Response: Decodable, Sendable {
            let key: String
            let value: FabushiJSONValue?
        }
        let res: Response? = try? await coordinator.dispatchFenced(
            method: FabushiRPCNamespace.settings.method("localGet"),
            params: Params(key: key)
        )
        return res?.value
    }
}
