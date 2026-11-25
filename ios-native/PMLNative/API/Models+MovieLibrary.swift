import Foundation

/// Server-side paged movie-library query. The server owns filtering, facets and ordering.
public struct MovieLibraryPage: Codable {
    public let items: [MovieMetadata]
    public let total: Int
    public let makers: [String]
    public let actors: [String]
}

public struct ScrapeIssue: Codable, Identifiable, Hashable {
    public var id: String { "\(number)-\(updatedAt)" }
    public let number: String
    public let error: String
    public let category: String
    public let retryable: Bool
    public let updatedAt: Int64
    public let files: [UnrecognizedVideo]?
}

public struct ScrapeIssueCount: Codable, Identifiable, Hashable {
    public var id: String { category }
    public let category: String
    public let count: Int
}

public struct ScrapeIssuePage: Codable {
    public let items: [ScrapeIssue]
    public let total: Int
    public let counts: [ScrapeIssueCount]
}

public struct UnrecognizedVideo: Codable, Identifiable, Hashable {
    public let id: String
    public let fileName: String
    public let remotePath: String
}

public struct UnrecognizedVideoPage: Codable {
    public let items: [UnrecognizedVideo]
    public let total: Int
}

public struct MovieAssetPart: Codable, Identifiable, Hashable {
    public var id: String { assetId }
    public let assetId: String
    public let fileName: String
    public let remotePath: String
    public let sizeBytes: Int64
    public let durationSeconds: Double?
}

public struct RelatedMoviesGrouped: Codable {
    public let actorNames: [String]
    public let byActors: [MovieMetadata]
    public let makerName: String?
    public let byMaker: [MovieMetadata]
    public let actorBirthDates: [String: String]?
}

public struct MoviePerformer: Codable, Identifiable, Hashable {
    public let id: String
    public let name: String
    public let aliases: [String]
    public let avatarUrl: String?
    public let profileUrl: String?
    public let source: String
    public let status: String
    public let updatedAt: Int64
    public let movieCount: Int
    public let birthDate: String?
    public let heightCm: Int?
    public let measurements: String?
    public let hometown: String?
    public let hobby: String?
    public let bio: String?
    public let bloodType: String?
    public let debutYear: String?
    public let agency: String?
    public let latestReleaseDate: String?
    public let latestAddedAt: Int64?
}

public struct PerformerRecentRelease: Codable, Identifiable, Hashable {
    public var id: String { number }
    public let number: String
    public let title: String
    public let releaseDate: String?
    public let coverUrl: String?
    public let detailUrl: String
    public let inLibrary: Bool
    public let source: String
}

public struct PerformerRecentReleasesResult: Codable {
    public let performerId: String
    public let performerName: String
    public let sourceUrl: String?
    public let totalFetched: Int
    public let unownedCount: Int
    public let items: [PerformerRecentRelease]
}

public struct MovieMagnetItem: Codable, Identifiable, Hashable {
    public var id: String { magnet }
    public let title: String
    public let magnet: String
    public let size: String?
    public let date: String?
    public let tags: [String]
    public let seeders: Int?
    public let source: String
}

public struct MoviePerformerList: Codable {
    public let items: [MoviePerformer]
    public let total: Int
}

public struct MoviePerformerDetail: Codable {
    public let person: MoviePerformer
    public let movies: [MovieMetadata]
    public let total: Int
}

public struct MovieProfileCandidate: Codable {
    public let avatarUrl: String
    public let profileUrl: String
    public let name: String
}

public struct PerformerEnrichStats: Codable {
    public let totalPerformers: Int
    public let withAvatar: Int
    public let withProfileInfo: Int
    public let missingCount: Int
}

public struct PerformerEnrichProgress: Codable {
    public let running: Bool
    public let paused: Bool
    public let total: Int
    public let completed: Int
    public let enriched: Int
    public let failed: Int
    public let currentName: String?
    public let lastMessage: String?
}

public struct RemoteSyncReport: Codable {
    public let scannedDirs: Int
    public let addedFiles: Int
    public let updatedFiles: Int
    public let removedFiles: Int
    public let unboundMissingAssets: Int
    public let reboundExistingMovies: Int
    public let cleanedStaleFailures: Int
    public let pendingNewCount: Int
    public let pendingNewNumbers: [String]
}

public struct SubtitleSearchCandidate: Codable, Identifiable, Hashable {
    public var id: String { downloadUrl }
    public let title: String
    public let lang: String
    public let langLabel: String
    public let downloadUrl: String
    public let source: String
}

public struct SavedMovieSubtitle: Codable, Identifiable, Hashable {
    public let id: String
    public let number: String
    public let name: String
    public let lang: String
    public let source: String
    public let createdAt: Int64
}
