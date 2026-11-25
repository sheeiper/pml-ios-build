import Combine
import Foundation

/// 全局状态（纯服务器模式 + AList/115 严格直链播放加速）。
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var settings: RuntimeSettings
    @Published private(set) var settingsRevision = 0
    @Published var playbackError: String?

    /// 客户端短 TTL 直链解析缓存（45s），避免短时间内重复打开/切集反复阻塞等服务端 `resolve_direct_playback`。
    private var directURLCache: [String: (url: URL, fetchedAt: Date)] = [:]
    private static let directURLCacheTTL: TimeInterval = 45

    init(settings: RuntimeSettings = .load()) {
        self.settings = settings
        configureImageAuth()
    }

    /// 唯一数据源：远程私人媒体库服务器。
    var apiClient: PMLAPIClient? {
        guard let url = settings.normalizedServerURL else { return nil }
        return PMLAPIClient(
            configuration: .init(
                baseURL: url,
                username: settings.serverUser,
                password: settings.serverPassword
            )
        )
    }

    var isConfigured: Bool {
        settings.isConfigured
    }

    var isServerMode: Bool {
        settings.normalizedServerURL != nil
    }

    var coreStatusText: String {
        if let url = settings.normalizedServerURL {
            return "服务器 · \(url.absoluteString)"
        }
        return "未配置服务器"
    }

    func updateSettings(_ newValue: RuntimeSettings) {
        settings = newValue
        newValue.save()
        directURLCache.removeAll()
        configureImageAuth()
        settingsRevision += 1
    }

    private func configureImageAuth() {
        ImageCache.shared.setServerAuth(
            baseURL: settings.normalizedServerURL,
            username: settings.serverUser,
            password: settings.serverPassword
        )
    }

    /// 回前台恢复：重建网络会话并轻量探活。
    func reconnectIfNeeded() async {
        APISession.shared.reset()
        guard let client = apiClient else { return }
        _ = try? await client.health()
    }

    // MARK: - 播放地址解析

    /// 视频一律严格直链；经 DirectPlaybackPolicy 校验通过后，交给手机本机 LocalMediaProxy
    /// 缓存 AList `/d` 302 后的真实 115 CDN 直链并复用连接池，提升拖动与切集速度。
    func playbackSource(assetId: String?, remotePath: String?) async -> PlaybackSource? {
        playbackError = nil
        guard let client = apiClient else {
            playbackError = "尚未配置服务器连接，请先在设置中填写服务器地址与密码"
            return nil
        }
        let cacheKey = Self.directCacheKey(assetId: assetId, remotePath: remotePath)
        do {
            let directURL: URL
            if let cacheKey,
               let cached = directURLCache[cacheKey],
               Date().timeIntervalSince(cached.fetchedAt) < Self.directURLCacheTTL {
                directURL = cached.url
            } else {
                let resolved = try await client.directPlaybackURL(
                    assetId: assetId,
                    remotePath: remotePath,
                    alistURL: settings.normalizedAlistURL
                )
                guard DirectPlaybackPolicy.allows(
                    resolved,
                    serverURL: settings.normalizedServerURL,
                    coreURL: nil,
                    alistURL: settings.normalizedAlistURL
                ) else {
                    playbackError = "解析结果不是安全的媒体直链，已拒绝服务器中转"
                    return nil
                }
                if let cacheKey {
                    if directURLCache.count > 128 {
                        let cutoff = Date().addingTimeInterval(-Self.directURLCacheTTL)
                        directURLCache = directURLCache.filter { $0.value.fetchedAt > cutoff }
                    }
                    directURLCache[cacheKey] = (resolved, Date())
                }
                directURL = resolved
            }
            if let proxied = await LocalMediaProxy.shared.register(directURL: directURL) {
                return PlaybackSource(url: proxied, fallbackURL: nil, label: "严格直连")
            }
            return PlaybackSource(url: directURL, fallbackURL: nil, label: "严格直连")
        } catch {
            if !Task.isCancelled {
                let detail = error.localizedDescription.trimmingCharacters(in: .whitespacesAndNewlines)
                playbackError = detail.isEmpty
                    ? "直链解析失败，请检查服务端 AList 配置及存储登录状态；不会回退中转。"
                    : "直链解析失败：\(detail)；不会回退中转。"
            }
            return nil
        }
    }

    func invalidatePlaybackCache(assetId: String?, remotePath: String? = nil, currentURL: URL? = nil) {
        if let key = Self.directCacheKey(assetId: assetId, remotePath: remotePath) {
            directURLCache.removeValue(forKey: key)
        }
        if let currentURL {
            LocalMediaProxy.shared.invalidateResolvedURL(for: currentURL)
        }
    }

    private static func directCacheKey(assetId: String?, remotePath: String?) -> String? {
        if let assetId, !assetId.isEmpty { return "id:\(assetId)" }
        if let remotePath, !remotePath.isEmpty { return "path:\(remotePath)" }
        return nil
    }

    func allowsDirectPlayback(_ url: URL) -> Bool {
        LocalMediaProxy.shared.isRegisteredLocalProxyURL(url)
            || DirectPlaybackPolicy.allows(
                url,
                serverURL: settings.normalizedServerURL,
                coreURL: nil,
                alistURL: settings.normalizedAlistURL
            )
    }

    /// 兼容旧播放器调用；视频请求绝不附带应用 Basic Auth 或 AList 管理凭证。
    func playbackHeaders(for url: URL) -> [String: String] { [:] }

    /// 图片/漫画走已鉴权的服务器图片通道；只有视频严格走 AList 直链。
    func imageSource(assetId: String?, remotePath: String?) async -> PlaybackSource? {
        guard let client = apiClient, let assetId,
              let url = try? await client.mediaSourceURL(assetId: assetId) else { return nil }
        return PlaybackSource(url: url, fallbackURL: nil, label: "服务器图片")
    }

    func proxyScraperImageURL(rawUrl: String) -> URL? {
        apiClient?.proxyScraperImageURL(rawUrl: rawUrl)
    }

    /// 把 CD2 路径按前缀映射到 AList，并拼出 `/d` 直链。
    func alistDirectURL(remotePath: String) -> URL? {
        guard let base = settings.normalizedAlistURL else { return nil }
        var cd2Prefix = settings.cd2Prefix.trimmingCharacters(in: .whitespaces)
        var alistPrefix = settings.alistPrefix.trimmingCharacters(in: .whitespaces)
        while cd2Prefix.count > 1 && cd2Prefix.hasSuffix("/") { cd2Prefix.removeLast() }
        while alistPrefix.count > 1 && alistPrefix.hasSuffix("/") { alistPrefix.removeLast() }
        guard !cd2Prefix.isEmpty,
              remotePath == cd2Prefix || remotePath.hasPrefix(cd2Prefix + "/") else { return nil }

        let mapped = alistPrefix + String(remotePath.dropFirst(cd2Prefix.count))
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "?#")
        let encoded = mapped
            .split(separator: "/", omittingEmptySubsequences: false)
            .map { String($0).addingPercentEncoding(withAllowedCharacters: allowed) ?? String($0) }
            .joined(separator: "/")
        return URL(string: base.absoluteString + "/d" + encoded)
    }
}
