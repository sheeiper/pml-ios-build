import Foundation

// MARK: - P0 用到的数据结构

struct ThumbnailInput: Decodable {
    let version: String
    let remotePath: String
    let mediaType: String
    let source: String?
    let cached: AssetThumbnail?
    let cachedUrl: String?
}

struct RemoteSearchPage: Decodable {
    let items: [RemoteBrowserEntry]
    let total: Int64
    let nextCursor: String?
}
