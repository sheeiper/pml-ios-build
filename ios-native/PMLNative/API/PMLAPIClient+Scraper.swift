import Foundation

extension PMLAPIClient {
    func scrapeAsset(assetId: String, fileName: String? = nil, remotePath: String? = nil, number: String? = nil, force: Bool = false) async throws -> MovieMetadata {
        var args: [String: Any] = [
            "assetId": assetId,
            "force": force
        ]
        if let number, !number.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { args["number"] = number }
        if let fileName = fileName { args["fileName"] = fileName }
        if let remotePath = remotePath { args["remotePath"] = remotePath }
        return try await command("scrape_asset", arguments: args, as: MovieMetadata.self)
    }

    func getMovieMetadata(number: String) async throws -> MovieMetadata? {
        return try await command("get_movie_metadata", arguments: ["number": number], as: MovieMetadata?.self)
    }

    func listMovieMetadata(limit: Int = 50, offset: Int = 0) async throws -> [MovieMetadata] {
        return try await command("list_movie_metadata", arguments: ["limit": limit, "offset": offset], as: [MovieMetadata].self)
    }

    func getRelatedMovies(number: String, limit: Int = 12) async throws -> [MovieMetadata] {
        return try await command("get_related_movies", arguments: ["number": number, "limit": limit], as: [MovieMetadata].self)
    }

    func getScrapeWhitelist() async throws -> [String] {
        return try await command("get_scrape_whitelist", as: [String].self)
    }

    func setScrapeWhitelist(paths: [String]) async throws {
        try await perform("set_scrape_whitelist", arguments: ["paths": paths])
    }

    func getAlbumDetails(dirPath: String) async throws -> AlbumInfo {
        return try await command("get_album_details", arguments: ["dirPath": dirPath], as: AlbumInfo.self)
    }

    func scanCleanupItems(rootPath: String? = nil) async throws -> [CleanupItem] {
        var args: [String: Any] = [:]
        if let rootPath = rootPath { args["rootPath"] = rootPath }
        return try await command("scan_cleanup_items", arguments: args, as: [CleanupItem].self)
    }

    func deleteCleanupItems(assetIds: [String]) async throws -> CleanupReport {
        return try await command("delete_cleanup_items", arguments: ["assetIds": assetIds], as: CleanupReport.self)
    }

    func proxyScraperImageURL(rawUrl: String) -> URL? {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else { return nil }
        components.path = "/api/scraper/proxy-image"
        components.queryItems = [URLQueryItem(name: "url", value: rawUrl)]
        return components.url
    }

    func getBatchScrapeStats() async throws -> BatchScrapeStats {
        return try await command("get_batch_scrape_stats", as: BatchScrapeStats.self)
    }

    func getBatchScrapeProgress() async throws -> BatchScrapeProgress {
        return try await command("get_batch_scrape_progress", as: BatchScrapeProgress.self)
    }

    func startBatchScrape() async throws {
        try await perform("start_batch_scrape")
    }

    func pauseBatchScrape() async throws -> BatchScrapeProgress {
        return try await command("pause_batch_scrape", as: BatchScrapeProgress.self)
    }

    func resumeBatchScrape() async throws -> BatchScrapeProgress {
        return try await command("resume_batch_scrape", as: BatchScrapeProgress.self)
    }

    func stopBatchScrape() async throws -> BatchScrapeProgress {
        return try await command("stop_batch_scrape", as: BatchScrapeProgress.self)
    }
}
