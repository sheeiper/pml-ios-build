import Foundation

// MARK: - 媒体源配置

struct ProviderConfig: Codable, Equatable {
    var providerType: String
    var cd2Endpoint: String? = nil
    var cd2User: String? = nil
    var cd2Password: String? = nil
    var alistEndpoint: String? = nil
    var alistToken: String? = nil
}

struct LibraryStatus: Decodable {
    let configured: Bool
    let connected: Bool
    let endpoint: String?
    let username: String?
    let hasSavedPassword: Bool
    let hasSavedToken: Bool
    let authMode: String
    let message: String
}

// MARK: - 资料库

struct LibraryRoot: Decodable, Identifiable, Hashable {
    let id: String
    let kind: String
    let remotePath: String
    let createdAt: Int64

    var kindLabel: String { kind == "comics" ? "漫画" : "媒体" }
    var symbol: String { kind == "comics" ? "book.closed.fill" : "photo.stack.fill" }
}

struct RemoteBrowserEntry: Decodable, Identifiable, Hashable {
    let path: String
    let name: String
    let isDirectory: Bool
    let sizeBytes: Int64
    let modifiedAt: String?
    let etag: String?
    let assetId: String?
    let mediaType: String?
    let thumbnailSource: String?
    let coverAssetId: String?
    var durationSeconds: Int64? = nil
    var progressSeconds: Int64? = nil

    var id: String { path }

    var isImage: Bool {
        guard let mediaType else { return false }
        return mediaType == "image" || mediaType == "comicPage"
    }

    var isVideo: Bool { mediaType == "video" }
    var isComicArchive: Bool { mediaType == "comicArchive" || mediaType == "pdf" }
}

struct RemoteBrowserPage: Decodable {
    let items: [RemoteBrowserEntry]
    let nextCursor: String?
    let fromLocalCache: Bool?
}

struct AssetThumbnail: Decodable {
    let mimeType: String
    let dataBase64: String
}

struct ComicArchivePageInfo: Decodable {
    let index: Int
    let name: String
}

struct ComicArchiveInfo: Decodable {
    let pages: [ComicArchivePageInfo]
}

// MARK: - 媒体资产 / 短视频

struct LibraryAsset: Decodable, Identifiable {
    let id: String
    let rootId: String?
    let fileName: String
    let parentPath: String
    let remotePath: String
    let mediaType: String
    let sizeBytes: Int64
    let durationSeconds: Int64?
    let authorName: String?
    let authorId: String?
    let thumbnailAssetId: String?
}

struct ShortVideoFeedPage: Decodable {
    let items: [LibraryAsset]
    let nextCursor: String?
}
