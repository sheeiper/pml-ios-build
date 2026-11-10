import Foundation

// MARK: - M4 首页 / 创作者

struct LibraryOverview: Decodable {
    let roots: Int64
    let assets: Int64
    let images: Int64
    let videos: Int64
    let shortVideos: Int64
    let comics: Int64
    let authors: Int64
    let collections: Int64
    let totalBytes: Int64
    let directories: Int64
    let unplayed: Int64
    let missingThumbnails: Int64
    let lastScanTime: Int64?
    let duplicateGroups: Int64
}

struct LibraryAuthor: Decodable, Identifiable {
    let id: String
    let name: String
    let aliases: [String]
    let mediaCount: Int64
    let collectionCount: Int64
    let avatarAssetId: String?
    let coverAssetId: String?
    let bannerAssetId: String?
}

struct PlaybackHistoryItem: Decodable, Identifiable {
    let asset: LibraryAsset
    let positionSeconds: Double
    let updatedAt: Int64

    var id: String { asset.id }
}

// MARK: - M5 扫描 / 设置

struct ScanResult: Decodable {
    let rootsScanned: Int
    let filesIndexed: Int
    let message: String
}

struct ScanProgress: Decodable {
    let running: Bool
    let phase: String
    let percent: Double
    let message: String
    let filesFound: Int
    let filesIndexed: Int
    let directoriesScanned: Int
    let error: String?
}

struct CacheStats: Decodable {
    let thumbnailBytes: UInt64
    let comicBytes: UInt64
    let databaseBytes: UInt64
    let shortVideoHistoryCount: Int64
    let shortVideoCacheBytes: UInt64
    let shortVideoCacheMaxBytes: UInt64
    let totalIndexedFiles: Int64
    let totalIndexedDirectories: Int64
    let lastScanTime: Int64?
    let lastScanDurationSecs: UInt64?
}

struct AutomationSettings: Codable, Equatable {
    var scheduledScanEnabled: Bool
    var scheduledScanTime: String
    var scheduledScanAvoidActive: Bool
    var lastScanTime: Int64?
    var lastScanDurationSecs: UInt64?
    var shortVideoCacheMaxBytes: UInt64
}

struct AlistAccelConfig: Codable, Equatable {
    var enabled: Bool
    var apiUrl: String
    var username: String
    var password: String?
    var cd2Prefix: String
    var alistPrefix: String
}

struct AlistTestResult: Decodable {
    let ok: Bool
    let message: String
}

struct MediaStreamInfo: Decodable {
    let accelerated: Bool
    let provider: String
    let accelProvider: String?
}
