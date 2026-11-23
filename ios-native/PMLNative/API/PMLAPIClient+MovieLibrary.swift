import Foundation

extension PMLAPIClient {
    func queryMovieLibrary(query: String = "", maker: String = "", actor: String = "", sort: String = "release_date_desc", limit: Int = 50, offset: Int = 0) async throws -> MovieLibraryPage {
        try await command("query_movie_library", arguments: ["query": query, "maker": maker, "actor": actor, "sort": sort, "limit": limit, "offset": offset], as: MovieLibraryPage.self)
    }

    func getRandomFeaturedMovies(limit: Int = 20) async throws -> [MovieMetadata] {
        try await command("get_random_featured_movies", arguments: ["limit": limit], as: [MovieMetadata].self)
    }

    func getRelatedMoviesGrouped(number: String, limitPerGroup: Int = 8) async throws -> RelatedMoviesGrouped {
        try await command("get_related_movies_grouped", arguments: ["number": number, "limitPerGroup": limitPerGroup], as: RelatedMoviesGrouped.self)
    }

    func listMovieAssets(number: String) async throws -> [MovieAssetPart] {
        try await command("list_movie_assets", arguments: ["number": number], as: [MovieAssetPart].self)
    }

    func listScrapeIssues(category: String = "", query: String = "", limit: Int = 50, offset: Int = 0) async throws -> ScrapeIssuePage {
        try await command("list_scrape_issues", arguments: ["category": category, "query": query, "limit": limit, "offset": offset], as: ScrapeIssuePage.self)
    }

    func listUnrecognizedVideos(limit: Int = 50, offset: Int = 0) async throws -> UnrecognizedVideoPage {
        try await command("list_unrecognized_videos", arguments: ["limit": limit, "offset": offset], as: UnrecognizedVideoPage.self)
    }

    func ignoreUnrecognizedVideo(assetId: String) async throws {
        try await command("ignore_unrecognized_video", arguments: ["assetId": assetId])
    }

    func syncRemoteAndReconcile(autoScrape: Bool = false, mode: String = "fast", skipFailed: Bool = true) async throws -> RemoteSyncReport {
        try await command("sync_remote_and_reconcile", arguments: ["autoScrape": autoScrape, "mode": mode, "skipFailed": skipFailed], as: RemoteSyncReport.self)
    }

    func startFailedBatchScrape() async throws {
        try await command("start_batch_scrape_failed")
    }

    func startNewBatchScrape() async throws {
        try await command("start_batch_scrape_new")
    }

    func listMoviePerformers(
        query: String = "",
        sort: String = "latest_release",
        minMovies: Int? = nil,
        ageRange: String = "",
        heightRange: String = "",
        cup: String = "",
        profileFilter: String = "",
        limit: Int = 50,
        offset: Int = 0
    ) async throws -> MoviePerformerList {
        var args: [String: Any] = ["query": query, "sort": sort, "limit": limit, "offset": offset]
        if let minMovies, minMovies > 0 { args["minMovies"] = minMovies }
        if !ageRange.isEmpty { args["ageRange"] = ageRange }
        if !heightRange.isEmpty { args["heightRange"] = heightRange }
        if !cup.isEmpty { args["cup"] = cup }
        if !profileFilter.isEmpty { args["profileFilter"] = profileFilter }
        return try await command("list_movie_performers", arguments: args, as: MoviePerformerList.self)
    }

    func getPerformerRecentReleases(id: String, includeOwned: Bool = false, forceRefresh: Bool = false) async throws -> PerformerRecentReleasesResult {
        try await command(
            "get_performer_recent_releases",
            arguments: ["id": id, "includeOwned": includeOwned, "forceRefresh": forceRefresh],
            as: PerformerRecentReleasesResult.self
        )
    }

    func getMovieMagnets(number: String, detailUrl: String? = nil, forceRefresh: Bool = false) async throws -> [MovieMagnetItem] {
        var args: [String: Any] = ["number": number, "forceRefresh": forceRefresh]
        if let detailUrl, !detailUrl.isEmpty { args["detailUrl"] = detailUrl }
        return try await command("get_movie_magnets", arguments: args, as: [MovieMagnetItem].self)
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

    func enrichMoviePerformer(id: String, force: Bool = false) async throws -> MoviePerformer {
        try await command("enrich_movie_performer", arguments: ["id": id, "force": force], as: MoviePerformer.self)
    }

    func getPerformerEnrichStats() async throws -> PerformerEnrichStats {
        try await command("get_performer_enrich_stats", as: PerformerEnrichStats.self)
    }

    func getPerformerEnrichProgress() async throws -> PerformerEnrichProgress {
        try await command("get_performer_enrich_progress", as: PerformerEnrichProgress.self)
    }

    func startPerformerEnrich(forceAll: Bool = false) async throws -> PerformerEnrichProgress {
        try await command("start_performer_enrich", arguments: ["forceAll": forceAll], as: PerformerEnrichProgress.self)
    }

    func pausePerformerEnrich() async throws -> PerformerEnrichProgress {
        try await command("pause_performer_enrich", as: PerformerEnrichProgress.self)
    }

    func resumePerformerEnrich() async throws -> PerformerEnrichProgress {
        try await command("resume_performer_enrich", as: PerformerEnrichProgress.self)
    }

    func stopPerformerEnrich() async throws -> PerformerEnrichProgress {
        try await command("stop_performer_enrich", as: PerformerEnrichProgress.self)
    }

    func searchMovieSubtitles(number: String) async throws -> [SubtitleSearchCandidate] {
        try await command("search_movie_subtitles", arguments: ["number": number], as: [SubtitleSearchCandidate].self)
    }

    func downloadMovieSubtitle(number: String, name: String, lang: String, downloadUrl: String) async throws -> SavedMovieSubtitle {
        try await command(
            "download_movie_subtitle",
            arguments: [
                "number": number,
                "name": name,
                "lang": lang,
                "downloadUrl": downloadUrl
            ],
            as: SavedMovieSubtitle.self
        )
    }

    func listMovieSubtitles(number: String) async throws -> [SavedMovieSubtitle] {
        try await command("list_movie_subtitles", arguments: ["number": number], as: [SavedMovieSubtitle].self)
    }

    func deleteMovieSubtitle(id: String) async throws {
        try await command("delete_movie_subtitle", arguments: ["id": id])
    }
}
