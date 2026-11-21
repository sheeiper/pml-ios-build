import Combine
import Foundation

/// 全局状态：内嵌 Rust 内核 + 本地/服务器运行模式。
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var corePort: Int?
    @Published private(set) var coreError: String?
    @Published private(set) var settings: RuntimeSettings
    @Published private(set) var settingsRevision = 0
    @Published var playbackError: String?

    init(corePort: Int32, coreError: String?, settings: RuntimeSettings = .load()) {
        self.corePort = corePort > 0 ? Int(corePort) : nil
        self.coreError = corePort > 0 ? nil : coreError
        self.settings = settings
        configureImageAuth()
    }

    var coreBaseURL: URL? {
        guard let corePort else { return nil }
        return URL(string: "http://127.0.0.1:\(corePort)")
    }

    /// 本地内嵌内核客户端（媒体代理与本地模式数据都可用）。
    var localClient: PMLAPIClient? {
        guard let coreBaseURL else { return nil }
        return PMLAPIClient(configuration: .init(baseURL: coreBaseURL, username: "", password: ""))
    }

    /// 当前数据源：服务器模式走远程，其余走本地内核。
    var apiClient: PMLAPIClient? {
        if settings.mode == .server, let url = settings.normalizedServerURL {
            return PMLAPIClient(
                configuration: .init(
                    baseURL: url,
                    username: settings.serverUser,
                    password: settings.serverPassword
                )
            )
        }
        return localClient
    }

    var isServerMode: Bool {
        settings.mode == .server && settings.normalizedServerURL != nil
    }

    var coreStatusText: String {
        if settings.mode == .server {
            return "服务器模式 · \(settings.serverURL)"
        }
        if let corePort { return "运行中 · 127.0.0.1:\(corePort)" }
        if let coreError { return "未启动：\(coreError)" }
        return "未启动"
    }

    func updateSettings(_ newValue: RuntimeSettings) {
        settings = newValue
        newValue.save()
        configureImageAuth()
        settingsRevision += 1
    }

    private func configureImageAuth() {
        let base = settings.mode == .server ? settings.normalizedServerURL : nil
        ImageCache.shared.setServerAuth(
            baseURL: base,
            username: settings.serverUser,
            password: settings.serverPassword
        )
    }

    /// 回前台恢复：服务器模式只重建连接；本地模式额外允许重启内核实监听。
    func reconnectIfNeeded() async {
        APISession.shared.reset()
        guard let client = apiClient else { return }
        if settings.mode == .server {
            _ = try? await client.health()
            return
        }
        guard corePort != nil else { return }
        if (try? await client.health()) != nil {
            if let status = try? await client.libraryStatus(),
               status.configured, !status.connected {
                _ = try? await client.restoreConnection()
            }
            return
        }
        let newPort = pml_core_restart()
        if newPort > 0 {
            corePort = Int(newPort)
            coreError = nil
            APISession.shared.reset()
            for _ in 0..<10 {
                if let refreshed = localClient, (try? await refreshed.health()) != nil {
                    return
                }
                try? await Task.sleep(nanoseconds: 200_000_000)
            }
            coreError = "内核已重启，但本地 API 仍无法连接"
        } else if let raw = pml_core_last_error() {
            coreError = String(cString: raw)
        }
    }

    // MARK: - 播放地址解析

    /// 视频一律严格直链（包括旧 relay 设置、本地模式及只有 assetId 的影视入口）。
    /// 控制面只返回 AList raw_url；不注册 /media，不预读视频，不通过 LocalMediaProxy。
    func playbackSource(assetId: String?, remotePath: String?) async -> PlaybackSource? {
        playbackError = nil
        guard let client = apiClient else {
            playbackError = "无法连接直链解析服务；不会使用服务器中转"
            return nil
        }
        do {
            let url = try await client.directPlaybackURL(assetId: assetId, remotePath: remotePath)
            guard allowsDirectPlayback(url) else {
                playbackError = "解析结果不是安全的媒体直链，已拒绝服务器中转"
                return nil
            }
            return PlaybackSource(url: url, fallbackURL: nil, label: "严格直连")
        } catch {
            if !Task.isCancelled {
                playbackError = "直链解析失败，请检查服务端 AList 配置及存储登录状态；不会回退中转。"
            }
            return nil
        }
    }

    func allowsDirectPlayback(_ url: URL) -> Bool {
        DirectPlaybackPolicy.allows(url, serverURL: settings.normalizedServerURL, coreURL: coreBaseURL)
    }

    /// 兼容旧播放器调用；视频请求绝不附带应用 Basic Auth 或 AList 管理凭证。
    func playbackHeaders(for url: URL) -> [String: String] { [:] }

    /// 图片/漫画保留原有映射与服务器媒体行为，与视频严格直链策略隔离。
    func imageSource(assetId: String?, remotePath: String?) async -> PlaybackSource? {
        if settings.mode == .server, settings.routeMode != .relay,
           let remotePath, let direct = alistDirectURL(remotePath: remotePath) {
            return PlaybackSource(url: direct, fallbackURL: nil, label: nil)
        }
        guard let client = apiClient, let assetId,
              let url = try? await client.mediaSourceURL(assetId: assetId) else { return nil }
        return PlaybackSource(url: url, fallbackURL: nil, label: nil)
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
        // Encode each path segment; leave '/' separators intact while escaping '?'/'#'
        // so filenames cannot accidentally become query/fragment delimiters.
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "?#")
        let encoded = mapped
            .split(separator: "/", omittingEmptySubsequences: false)
            .map { String($0).addingPercentEncoding(withAllowedCharacters: allowed) ?? String($0) }
            .joined(separator: "/")
        return URL(string: base.absoluteString + "/d" + encoded)
    }

}
