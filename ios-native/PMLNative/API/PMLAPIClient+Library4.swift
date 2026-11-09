import Foundation

extension PMLAPIClient {
    // MARK: 收纳入库

    func curatorConfig() async throws -> CuratorConfigPayload {
        try await command("get_curator_config")
    }

    func saveCuratorConfig(pendingPath: String?, targetVaultRoot: String?) async throws {
        // Root paths are owned by the canonical V2 directory configuration.
        // Preserve the other two domains while updating curation destinations.
        var config = try await directoryConfig()
        config.pendingPath = pendingPath
        config.vaultPath = targetVaultRoot
        _ = try await saveDirectoryConfig(config)
    }

    func curateMoveItem(sourcePath: String, targetPath: String, isDirectory: Bool) async throws -> CurateMoveItemResult {
        try await command(
            "curate_move_item",
            arguments: [
                "sourcePath": sourcePath,
                "targetPath": targetPath,
                "isDirectory": isDirectory,
            ]
        )
    }

    func curateBatchMove(_ items: [(source: String, target: String, isDirectory: Bool)]) async throws -> CurateBatchMoveResult {
        let payload = items.map { item -> [String: Any] in
            [
                "sourcePath": item.source,
                "targetPath": item.target,
                "isDirectory": item.isDirectory,
            ]
        }
        return try await command(
            "curate_batch_move",
            arguments: ["items": payload, "delayMs": 120]
        )
    }

    func purgePendingAssets(pendingPath: String) async throws {
        try await perform("purge_pending_assets", arguments: ["pendingPath": pendingPath])
    }

    func deleteEmptyPendingBatch(batchPath: String) async throws {
        try await perform("delete_empty_pending_batch", arguments: ["batchPath": batchPath])
    }

    func rawMediaURL(remotePath: String) async throws -> URL {
        let raw: String = try await command("open_raw_media_source", arguments: ["remotePath": remotePath])
        return try resolve(raw)
    }

    // MARK: 分类 / 合集

    func libraryCategories(parentId: String? = nil, filter: String = "all") async throws -> [LibraryCategory] {
        var arguments: [String: Any] = ["filter": filter]
        if let parentId { arguments["parentId"] = parentId }
        return try await command("list_library_categories", arguments: arguments)
    }

    func libraryCollections(
        authorId: String? = nil,
        categoryId: String? = nil
    ) async throws -> [LibraryCollection] {
        var arguments: [String: Any] = [:]
        if let authorId { arguments["authorId"] = authorId }
        if let categoryId { arguments["categoryId"] = categoryId }
        return try await command("list_library_collections", arguments: arguments)
    }

    func libraryAssets(
        filter: String = "all",
        categoryId: String? = nil,
        collectionId: String? = nil,
        authorId: String? = nil,
        limit: Int = 200,
        offset: Int = 0
    ) async throws -> [LibraryAsset] {
        var arguments: [String: Any] = ["filter": filter, "limit": limit, "offset": offset]
        if let categoryId { arguments["categoryId"] = categoryId }
        if let collectionId { arguments["collectionId"] = collectionId }
        if let authorId { arguments["authorId"] = authorId }
        return try await command("list_library_assets", arguments: arguments)
    }

    // MARK: 作者 / 封面

    func updateAuthorAliases(authorId: String, aliases: [String]) async throws {
        try await perform("update_author_aliases", arguments: ["authorId": authorId, "aliases": aliases])
    }

    func setDirectoryCover(rootId: String, remotePath: String, coverAssetId: String) async throws {
        try await perform(
            "set_directory_cover",
            arguments: [
                "rootId": rootId,
                "remotePath": remotePath,
                "coverAssetId": coverAssetId,
            ]
        )
    }

    func setComicCoverPage(assetId: String, pageIndex: Int) async throws -> String {
        try await command(
            "set_comic_cover_page",
            arguments: ["assetId": assetId, "pageIndex": pageIndex]
        )
    }
}
