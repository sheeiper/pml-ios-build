import Foundation

/// 统一持有 URLSession：
/// - `session`：高优先级交互通道（目录浏览、播放进度、收藏等），8 并发连接，绝不被缩略图阻塞；
/// - `thumbnailSession`：独立低优先级缩略图通道，与目录浏览物理隔离，彻底消除加载封面拖慢打开文件夹的问题。
final class APISession {
    static let shared = APISession()

    private(set) var session: URLSession
    private(set) var thumbnailSession: URLSession

    private init() {
        session = Self.makeInteractive()
        thumbnailSession = Self.makeThumbnail()
    }

    static func makeInteractive() -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 20
        config.timeoutIntervalForResource = 60
        config.waitsForConnectivity = false
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.httpMaximumConnectionsPerHost = 8
        return URLSession(configuration: config)
    }

    static func makeThumbnail() -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 15
        config.timeoutIntervalForResource = 45
        config.waitsForConnectivity = false
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.httpMaximumConnectionsPerHost = 3
        config.networkServiceType = .background
        return URLSession(configuration: config)
    }

    func reset() {
        session.invalidateAndCancel()
        thumbnailSession.invalidateAndCancel()
        session = Self.makeInteractive()
        thumbnailSession = Self.makeThumbnail()
    }
}
