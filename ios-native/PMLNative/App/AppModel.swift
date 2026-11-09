import Combine
import Foundation

/// 全局状态：内嵌 Rust 内核 + 本地/服务器运行模式。
@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var corePort: Int?
    @Published private(set) var coreError: String?
    @Published private(set) var settings: RuntimeSettings
    @Published private(set) var settingsRevision = 0

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

    /// 服务器模式：优先 AList `/d` 直连；失败回退服务器 `/media`。
    func playbackSource(assetId: String?, remotePath: String?) async -> PlaybackSource? {
        if settings.mode == .server {
            let mode = settings.routeMode
            let direct = remotePath.flatMap { alistDirectURL(remotePath: $0) }
            let isMOV = remotePath?.lowercased().hasSuffix(".mov") == true

            // 自动模式：MOV 默认走服务器中转，稳定且首帧可控；直连需手动选择。
            if mode == .relay || (mode == .auto && isMOV) {
                if let relay = await serverMediaURL(assetId: assetId) {
                    return PlaybackSource(url: relay, fallbackURL: direct, label: "服务器中转")
                }
            }

            // 直连：把 AList /d 直接交给 AVPlayer，由播放器自己跟随 302。
            // 不要先取最终 CDN URL 再复用：115 链接可能被首次请求消费/绑定。
            if let direct {
                return PlaybackSource(url: direct, fallbackURL: nil, label: "AList 直连")
            }

            if let relay = await serverMediaURL(assetId: assetId) {
                return PlaybackSource(url: relay, fallbackURL: nil, label: "服务器中转")
            }
            return nil
        }

        guard let client = localClient, let assetId,
              let url = try? await client.mediaSourceURL(assetId: assetId) else {
            return nil
        }
        return PlaybackSource(url: url, fallbackURL: nil, label: nil)
    }
    /// AVPlayer 访问服务器 /media 时补 Basic 认证；直连 AList/CDN 不加，避免泄露。
    func playbackHeaders(for url: URL) -> [String: String] {
        guard settings.mode == .server,
              let base = settings.normalizedServerURL,
              let targetHost = url.host,
              let baseHost = base.host,
              targetHost == baseHost else { return [:] }
        let targetPort = url.port ?? (url.scheme == "https" ? 443 : 80)
        let basePort = base.port ?? (base.scheme == "https" ? 443 : 80)
        guard targetPort == basePort else { return [:] }
        let raw = "\(settings.serverUser):\(settings.serverPassword)"
        let token = Data(raw.utf8).base64EncodedString()
        return ["Authorization": "Basic \(token)"]
    }
    /// 播放失败时再取服务器中转地址（不阻塞首帧）。
    func fallbackMediaURL(assetId: String?) async -> URL? {
        guard let assetId, let client = apiClient else { return nil }
        return try? await client.mediaSourceURL(assetId: assetId)
    }

    private func serverMediaURL(assetId: String?) async -> URL? {
        await fallbackMediaURL(assetId: assetId)
    }

    /// 把 CD2 路径按前缀映射到 AList，并拼出 `/d` 直链。
    func alistDirectURL(remotePath: String) -> URL? {
        guard let base = settings.normalizedAlistURL else { return nil }
        var cd2Prefix = settings.cd2Prefix.trimmingCharacters(in: .whitespaces)
        var alistPrefix = settings.alistPrefix.trimmingCharacters(in: .whitespaces)
        while cd2Prefix.count > 1 && cd2Prefix.hasSuffix("/") { cd2Prefix.removeLast() }
        while alistPrefix.count > 1 && alistPrefix.hasSuffix("/") { alistPrefix.removeLast() }
        guard !cd2Prefix.isEmpty, remotePath.hasPrefix(cd2Prefix) else { return nil }

        let mapped = alistPrefix + String(remotePath.dropFirst(cd2Prefix.count))
        let encoded = mapped
            .split(separator: "/", omittingEmptySubsequences: false)
            .map { String($0).addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? String($0) }
            .joined(separator: "/")
        return URL(string: base.absoluteString + "/d" + encoded)
    }

    /// 预检直链：跟随 302 后能拿到 200/206 才使用，避免播放器黑屏。
    private func preflight(_ url: URL) async -> Bool {
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("bytes=0-0", forHTTPHeaderField: "Range")
        request.timeoutInterval = 6
        do {
            let (_, response) = try await APISession.shared.session.data(for: request)
            guard let http = response as? HTTPURLResponse else { return false }
            return http.statusCode == 200 || http.statusCode == 206
        } catch {
            return false
        }
    }
}
