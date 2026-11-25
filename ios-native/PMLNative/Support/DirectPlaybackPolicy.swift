import Foundation

enum DirectPlaybackPolicy {
    static let failureMessage = "直链播放失败：链接已失效、设备无法访问或格式不受支持。请检查 AList 登录与直链配置后重试；不会切换服务器中转。"

    static func allows(_ url: URL, serverURL: URL?, coreURL: URL?, alistURL: URL? = nil, lanAlistURL: URL? = nil) -> Bool {
        guard let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme),
              let host = url.host?.lowercased(), !host.isEmpty,
              url.user == nil, url.password == nil else { return false }
        guard host != "localhost", host != "::1", host != "[::1]",
              host != "0.0.0.0", !host.hasPrefix("127.") else { return false }
        let path = (url.path.removingPercentEncoding ?? url.path).lowercased()
        let segments = path.split(separator: "/")
        guard !segments.contains("media"), !segments.contains("dav"),
              !segments.contains("p"), !segments.contains("api") else { return false }
        // AList may be mounted on the same origin as PML. Permit only the
        // configured AList origin's /d route; never permit PML /d by accident.
        let sameServer = [serverURL, coreURL].compactMap { $0 }.contains { sameOrigin(url, $0) }
        if sameServer && !isAListDownloadURL(url, base: alistURL) && !isAListDownloadURL(url, base: lanAlistURL) {
            return false
        }
        return true
    }

    private static func isAListDownloadURL(_ url: URL, base: URL?) -> Bool {
        guard let base, sameOrigin(url, base) else { return false }
        var prefix = base.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if prefix.caseInsensitiveCompare("dav") == .orderedSame || prefix.caseInsensitiveCompare("api") == .orderedSame {
            prefix = ""
        } else if prefix.hasSuffix("/dav") || prefix.hasSuffix("/api") {
            prefix = String(prefix.dropLast(4)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        }
        let expected = prefix.isEmpty ? "/d/" : "/\(prefix)/d/"
        return url.path.hasPrefix(expected)
    }

    private static func sameOrigin(_ lhs: URL, _ rhs: URL) -> Bool {
        func port(_ url: URL) -> Int { url.port ?? (url.scheme?.lowercased() == "https" ? 443 : 80) }
        return lhs.scheme?.lowercased() == rhs.scheme?.lowercased()
            && lhs.host?.lowercased() == rhs.host?.lowercased() && port(lhs) == port(rhs)
    }
}
