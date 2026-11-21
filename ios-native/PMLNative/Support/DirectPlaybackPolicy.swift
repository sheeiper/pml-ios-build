import Foundation

/// Pure policy shared by every video entry point. No route setting can opt video into relay.
enum DirectPlaybackPolicy {
    static let failureMessage = "直链播放失败：链接已失效、设备无法访问或格式不受支持。请检查 AList 登录与直链配置后重试；不会切换服务器中转。"

    static func allows(_ url: URL, serverURL: URL?, coreURL: URL?) -> Bool {
        guard let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
              let host = url.host?.lowercased(), !host.isEmpty,
              url.user == nil, url.password == nil else { return false }
        // LocalMediaProxy and embedded /media are not direct playback either.
        guard host != "localhost", host != "::1", host != "[::1]",
              host != "0.0.0.0", !host.hasPrefix("127.") else { return false }
        let path = (url.path.removingPercentEncoding ?? url.path).lowercased()
        let segments = path.split(separator: "/")
        guard !segments.contains("media"), !segments.contains("dav"),
              !segments.contains("api") else { return false }
        // Same host at a different port is valid (e.g. AList vs PML).
        for base in [serverURL, coreURL].compactMap({ $0 }) {
            if sameOrigin(url, base) { return false }
        }
        return true
    }

    private static func sameOrigin(_ lhs: URL, _ rhs: URL) -> Bool {
        func port(_ url: URL) -> Int { url.port ?? (url.scheme?.lowercased() == "https" ? 443 : 80) }
        return lhs.scheme?.lowercased() == rhs.scheme?.lowercased()
            && lhs.host?.lowercased() == rhs.host?.lowercased() && port(lhs) == port(rhs)
    }
}
