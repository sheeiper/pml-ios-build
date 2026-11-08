import Foundation

struct DuplicateAsset: Decodable, Identifiable {
    let id: String
    let rootId: String
    let fileName: String
    let remotePath: String
    let parentPath: String
    let mediaType: String
    let sizeBytes: Int64
    let modifiedAt: String?
}

struct DuplicateGroup: Decodable, Identifiable {
    let hash: String
    let sizeBytes: Int64
    let assets: [DuplicateAsset]

    var id: String { hash }
}

struct DuplicateScanProgress: Decodable {
    let running: Bool
    let phase: String
    let candidatesTotal: Int
    let processed: Int
    let duplicateGroups: Int
    let matchedFiles: Int
    let percent: Double
    let message: String
    let error: String?
}

struct PackInboxResult: Decodable {
    let movedCount: Int
    let targetBatchPath: String
}
