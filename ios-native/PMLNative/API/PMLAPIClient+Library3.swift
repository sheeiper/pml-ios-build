import Foundation

extension PMLAPIClient {
    // MARK: 缩略图

    func thumbnailInput(assetId: String, force: Bool = false) async throws -> ThumbnailInput {
        var arguments: [String: Any] = ["assetId": assetId]
        if force { arguments["force"] = true }
        return try await command("get_thumbnail_input", arguments: arguments)
    }

    func saveAssetThumbnail(
        assetId: String,
        mimeType: String,
        dataBase64: String,
        expectedVersion: String?
    ) async throws {
        var arguments: [String: Any] = [
            "assetId": assetId,
            "mimeType": mimeType,
            "dataBase64": dataBase64,
        ]
        if let expectedVersion { arguments["expectedVersion"] = expectedVersion }
        try await perform("save_asset_thumbnail", arguments: arguments)
    }

    // MARK: 收藏

    func toggleFavorite(assetId: String) async throws -> Bool {
        try await command("toggle_favorite", arguments: ["assetId": assetId])
    }

    func favoriteAssets(limit: Int = 100, offset: Int = 0) async throws -> [LibraryAsset] {
        try await command("list_favorite_assets", arguments: ["limit": limit, "offset": offset])
    }

    func favoriteIds() async throws -> [String] {
        try await command("list_favorite_ids")
    }

    // MARK: 搜索

    func searchRemoteFiles(
        rootId: String,
        parentPath: String,
        keywords: String,
        cursor: String? = nil
    ) async throws -> RemoteSearchPage {
        var arguments: [String: Any] = [
            "rootId": rootId,
            "parentPath": parentPath,
            "keywords": keywords,
        ]
        if let cursor { arguments["cursor"] = cursor }
        return try await command("search_remote_files", arguments: arguments)
    }

    // MARK: 播放进度

    func playbackProgress(assetId: String) async throws -> Double {
        try await command("get_asset_playback_progress", arguments: ["assetId": assetId])
    }
}
