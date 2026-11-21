import Foundation

extension PMLAPIClient {
    func queryMovieLibrary(query: String = "", maker: String = "", actor: String = "", sort: String = "updated_at", limit: Int = 50, offset: Int = 0) async throws -> MovieLibraryPage {
        try await command("query_movie_library", arguments: ["query": query, "maker": maker, "actor": actor, "sort": sort, "limit": limit, "offset": offset], as: MovieLibraryPage.self)
    }

    func listScrapeIssues(category: String = "", query: String = "", limit: Int = 50, offset: Int = 0) async throws -> ScrapeIssuePage {
        try await command("list_scrape_issues", arguments: ["category": category, "query": query, "limit": limit, "offset": offset], as: ScrapeIssuePage.self)
    }

    func listUnrecognizedVideos(limit: Int = 50, offset: Int = 0) async throws -> UnrecognizedVideoPage {
        try await command("list_unrecognized_videos", arguments: ["limit": limit, "offset": offset], as: UnrecognizedVideoPage.self)
    }

    func listMoviePerformers(query: String = "", limit: Int = 50, offset: Int = 0) async throws -> MoviePerformerList {
        try await command("list_movie_performers", arguments: ["query": query, "limit": limit, "offset": offset], as: MoviePerformerList.self)
    }

    func getMoviePerformer(id: String) async throws -> MoviePerformerDetail {
        try await command("get_movie_performer", arguments: ["id": id], as: MoviePerformerDetail.self)
    }

    func updateMoviePerformer(id: String, name: String, aliases: [String], avatarURL: String?, profileURL: String?) async throws -> MoviePerformer {
        var arguments: [String: Any] = ["id": id, "name": name, "aliases": aliases]
        arguments["avatarUrl"] = avatarURL ?? NSNull()
        arguments["profileUrl"] = profileURL ?? NSNull()
        return try await command("update_movie_performer", arguments: arguments, as: MoviePerformer.self)
    }

    func setMoviePerformerStatus(id: String, status: String) async throws -> MoviePerformer {
        try await command("set_movie_performer_status", arguments: ["id": id, "status": status], as: MoviePerformer.self)
    }

    func previewMoviePerformerProfile(id: String, profileURL: String) async throws -> MovieProfileCandidate {
        try await command("preview_movie_performer_profile", arguments: ["id": id, "profileUrl": profileURL], as: MovieProfileCandidate.self)
    }
}
