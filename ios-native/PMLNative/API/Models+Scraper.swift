import Foundation

public struct MovieMetadata: Codable, Identifiable, Hashable {
    public var id: String { number }
    public let number: String
    public let assetId: String?
    public let title: String
    public let originalTitle: String?
    public let actors: [String]
    public let maker: String?
    public let label: String?
    public let series: String?
    public let director: String?
    public let releaseDate: String?
    public let durationMinutes: Int?
    public let coverUrl: String?
    public let sampleImages: [String]
    public let tags: [String]
    public let source: String
    public let updatedAt: Int64

    public init(
        number: String,
        assetId: String? = nil,
        title: String,
        originalTitle: String? = nil,
        actors: [String] = [],
        maker: String? = nil,
        label: String? = nil,
        series: String? = nil,
        director: String? = nil,
        releaseDate: String? = nil,
        durationMinutes: Int? = nil,
        coverUrl: String? = nil,
        sampleImages: [String] = [],
        tags: [String] = [],
        source: String = "javdb",
        updatedAt: Int64 = 0
    ) {
        self.number = number
        self.assetId = assetId
        self.title = title
        self.originalTitle = originalTitle
        self.actors = actors
        self.maker = maker
        self.label = label
        self.series = series
        self.director = director
        self.releaseDate = releaseDate
        self.durationMinutes = durationMinutes
        self.coverUrl = coverUrl
        self.sampleImages = sampleImages
        self.tags = tags
        self.source = source
        self.updatedAt = updatedAt
    }
}

public struct AlbumItem: Codable, Identifiable, Hashable {
    public var id: String { assetId }
    public let assetId: String
    public let fileName: String
    public let remotePath: String
    public let sizeBytes: Int64
}

public struct AlbumInfo: Codable, Hashable {
    public let dirPath: String
    public let title: String
    public let imageCount: Int
    public let totalSizeBytes: Int64
    public let coverAssetId: String?
    public let previewAssetIds: [String]
    public let items: [AlbumItem]
}

public struct CleanupItem: Codable, Identifiable, Hashable {
    public var id: String { assetId }
    public let assetId: String
    public let fileName: String
    public let remotePath: String
    public let sizeBytes: Int64
    public let category: String // "text" | "image" | "video"
    public let reason: String
}

public struct CleanupReport: Codable {
    public let deletedCount: Int
    public let freedBytes: Int64
}

public struct BatchScrapeProgress: Codable {
    public let running: Bool
    public let paused: Bool
    public let total: Int
    public let completed: Int
    public let succeeded: Int
    public let failed: Int
    public let currentNumber: String?
    public let currentActorStatus: String?
    public let lastError: String?
}

public struct BatchScrapeStats: Codable {
    public let totalWhitelistedVideos: Int
    public let distinctNumbers: Int
    public let alreadyScraped: Int
    public let remainingToScrape: Int
    public let failedCount: Int?
}

public struct PerformerSourceCandidate: Codable, Identifiable {
    public var id: String { source }
    public let source: String
    public let sourceLabel: String
    public let name: String
    public let aliases: [String]
    public let avatarUrl: String?
    public let profileUrl: String?
    public let birthDate: String?
    public let heightCm: Int?
    public let measurements: String?
    public let hometown: String?
    public let hobby: String?
    public let bloodType: String?
    public let debutYear: String?
    public let agency: String?
    public let bio: String?
}

public struct SyncDiffFileItem: Codable, Identifiable {
    public var id: String { assetId + "|" + remotePath }
    public let assetId: String
    public let fileName: String
    public let remotePath: String
}

public struct MovieSyncDiffSummary: Codable {
    public let runId: String?
    public let mode: String
    public let ranAt: Int64?
    public let scannedDirs: Int
    public let addedCount: Int
    public let removedCount: Int
    public let added: [SyncDiffFileItem]
    public let removed: [SyncDiffFileItem]
}

public struct MovieExtractSummary: Codable {
    public let scope: String
    public let scannedVideos: Int
    public let extractedNumbers: Int
    public let alreadyInLibrary: Int
    public let pendingQueueCount: Int
    public let unrecognizedCount: Int
    public let pendingNumbersPreview: [String]
}
