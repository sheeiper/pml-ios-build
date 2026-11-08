import Foundation

// MARK: - P1 整理

struct CuratorConfigPayload: Decodable {
    let filterWords: [String]?
    let pendingPath: String?
    let targetVaultRoot: String?
}

struct CurateMoveItemResult: Decodable {
    let success: Bool
    let status: String
    let message: String
    let sourcePath: String
    let targetPath: String
}

struct CurateBatchMoveResult: Decodable {
    let successCount: Int
    let skippedCount: Int
    let failedCount: Int
    let aborted: Bool
    let abortReason: String?
    let results: [CurateMoveItemResult]
}

struct LibraryCategory: Decodable, Identifiable {
    let id: String
    let name: String
    let parentId: String?
    let mediaCount: Int64
    let authorCount: Int64
    let collectionCount: Int64
}

struct LibraryCollection: Decodable, Identifiable {
    let id: String
    let name: String
    let authorId: String?
    let authorName: String?
    let rootId: String
    let mediaCount: Int64
    let imageCount: Int64
    let videoCount: Int64
    let comicCount: Int64
}

/// 待收纳的单个条目（来自远端目录，尚未纳管）。
struct PendingItem: Identifiable {
    let entry: RemoteBrowserEntry
    var targetDir: String?
    var suggestedAuthor: String?

    var id: String { entry.path }
}
