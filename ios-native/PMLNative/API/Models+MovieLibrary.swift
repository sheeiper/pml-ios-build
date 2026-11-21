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
