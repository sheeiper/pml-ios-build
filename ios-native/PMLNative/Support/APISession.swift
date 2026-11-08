import Foundation

/// 统一持有 URLSession。前后台切换后旧连接可能失效，reset 会丢弃连接池重新建连。
final class APISession {
    static let shared = APISession()

    private(set) var session: URLSession

    private init() {
        session = Self.make()
    }

    static func make() -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20
        config.timeoutIntervalForResource = 60
        config.waitsForConnectivity = false
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.httpMaximumConnectionsPerHost = 4
        return URLSession(configuration: config)
    }

    func reset() {
        session.invalidateAndCancel()
        session = Self.make()
    }
}
