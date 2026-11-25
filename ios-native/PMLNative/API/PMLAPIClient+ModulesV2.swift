import Foundation

// MARK: - V2 多模块 API（创作者 / 合集 / 图集 / 模块管理）

extension PMLAPIClient {
    // MARK: 缩略图与封面地址规则

    /// 统一解析 V2 封面/缩略图 URL：
    /// - 显式上传封面（author_* / banner_*）：`/api/covers/{id}/{id}.jpg`
    /// - 图集压缩包（isArchive == true）：`/api/comic-pages/{id}/0`
    /// - 普通视频/图片资产（asset-*）：`/thumbnail/thumb_{id}`（必须带 thumb_ 前缀）
    func v2CoverURL(assetId: String?, isArchive: Bool = false) -> URL? {
        guard let raw = assetId?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty else {
            return nil
        }
        if raw.hasPrefix("author_") || raw.hasPrefix("banner_") {
            let encoded = raw.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? raw
            return URL(string: "/api/covers/\(encoded)/\(encoded).jpg", relativeTo: configuration.baseURL)?.absoluteURL
        }
        if isArchive {
            return comicPageURL(assetId: raw, pageIndex: 0)
        }
        let key = raw.hasPrefix("thumb_") ? raw : "thumb_\(raw)"
        let encoded = key.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? key
        return URL(string: "/thumbnail/\(encoded)", relativeTo: configuration.baseURL)?.absoluteURL
    }

    // MARK: 多模块配置与扫描

    func listLibraryModules() async throws -> [LibraryModuleInfo] {
        try await command("list_library_modules")
    }

    func upsertLibraryModule(
        id: String,
        kind: String,
        name: String,
        remotePath: String,
        domain: String? = nil,
        enabled: Bool? = nil,
        sortOrder: Int64? = nil
    ) async throws -> LibraryModuleInfo {
        var args: [String: Any] = [
            "id": id,
            "kind": kind,
            "name": name,
            "remotePath": remotePath,
        ]
        if let domain { args["domain"] = domain }
        if let enabled { args["enabled"] = enabled }
        if let sortOrder { args["sortOrder"] = sortOrder }
        return try await command("upsert_library_module", arguments: args)
    }

    func scanLibraryModule(moduleId: String, full: Bool = false) async throws -> ModuleScanResult {
        try await command("scan_library_module", arguments: ["moduleId": moduleId, "full": full])
    }

    func rebuildModuleIndexes(moduleId: String? = nil) async throws -> [ModuleRebuildSummary] {
        var args: [String: Any] = [:]
        if let moduleId, !moduleId.isEmpty { args["moduleId"] = moduleId }
        return try await command("rebuild_module_indexes", arguments: args)
    }

    // MARK: 实体级收藏（影片 / 演员 / 创作者）

    func toggleEntityFavorite(type: EntityFavoriteType, entityId: String) async throws -> Bool {
        try await command("toggle_entity_favorite", arguments: [
            "entityType": type.rawValue,
            "entityId": entityId,
        ])
    }

    func listEntityFavoriteIds() async throws -> EntityFavoriteIds {
        try await command("list_entity_favorite_ids")
    }

    // MARK: 创作者 V2

    func listCreatorStudios() async throws -> [CreatorStudioStat] {
        try await command("list_creator_studios")
    }

    func listCreatorsV2(
        studio: String? = nil,
        query: String? = nil,
        sort: String = "count",
        onlyFavorites: Bool = false
    ) async throws -> [CreatorSummaryV2] {
        var args: [String: Any] = ["sort": sort]
        if let studio, !studio.isEmpty { args["studio"] = studio }
        if let query, !query.isEmpty { args["query"] = query }
        if onlyFavorites { args["onlyFavorites"] = true }
        return try await command("list_creators_v2", arguments: args)
    }

    func browseCreatorFolder(
        authorId: String,
        studio: String? = nil,
        subPath: String? = nil,
        query: String? = nil,
        sort: String = "name",
        limit: Int = 120,
        offset: Int = 0
    ) async throws -> CreatorFolderResponse {
        var args: [String: Any] = [
            "authorId": authorId,
            "sort": sort,
            "limit": limit,
            "offset": offset,
        ]
        if let studio, !studio.isEmpty { args["studio"] = studio }
        if let subPath, !subPath.isEmpty { args["subPath"] = subPath }
        if let query, !query.isEmpty { args["query"] = query }
        return try await command("browse_creator_folder", arguments: args)
    }

    func updateCreatorProfile(
        authorId: String,
        aliases: [String]? = nil,
        bio: String? = nil,
        socialLinks: [String: String]? = nil
    ) async throws -> CreatorSummaryV2 {
        var args: [String: Any] = ["authorId": authorId]
        if let aliases { args["aliases"] = aliases }
        if let bio { args["bio"] = bio }
        if let socialLinks { args["socialLinks"] = socialLinks }
        return try await command("update_creator_profile", arguments: args)
    }

    func setAuthorAvatar(authorId: String, assetId: String) async throws {
        try await perform("set_author_avatar", arguments: ["authorId": authorId, "assetId": assetId])
    }

    func setAuthorBanner(authorId: String, assetId: String) async throws {
        try await perform("set_author_banner", arguments: ["authorId": authorId, "assetId": assetId])
    }

    func listCreatorMergeSuggestions() async throws -> [CreatorMergeSuggestion] {
        try await command("list_creator_merge_suggestions")
    }

    func ignoreCreatorMergePair(authorIdA: String, authorIdB: String) async throws {
        try await perform(
            "ignore_creator_merge_pair",
            arguments: ["authorIdA": authorIdA, "authorIdB": authorIdB]
        )
    }

    func verifyCreatorMerge(
        keepAuthorId: String,
        mergedAuthorId: String,
        keepAlias: Bool = true
    ) async throws -> VerifyCreatorMergeResult {
        try await command(
            "verify_creator_merge",
            arguments: [
                "keepAuthorId": keepAuthorId,
                "mergedAuthorId": mergedAuthorId,
                "keepAlias": keepAlias,
            ]
        )
    }

    // MARK: 专题合集 V2

    func listCollectionChannels() async throws -> [CollectionChannelStat] {
        try await command("list_collection_channels")
    }

    func listCollectionAlbums(
        channel: String? = nil,
        query: String? = nil,
        sort: String = "count",
        limit: Int = 80,
        offset: Int = 0
    ) async throws -> CollectionAlbumsPage {
        var args: [String: Any] = [
            "sort": sort,
            "limit": limit,
            "offset": offset,
        ]
        if let channel, !channel.isEmpty { args["channel"] = channel }
        if let query, !query.isEmpty { args["query"] = query }
        return try await command("list_collection_albums", arguments: args)
    }

    func browseCollectionFolder(
        albumId: String,
        subPath: String? = nil,
        query: String? = nil,
        sort: String = "name",
        limit: Int = 120,
        offset: Int = 0
    ) async throws -> CollectionFolderResponse {
        var args: [String: Any] = [
            "albumId": albumId,
            "sort": sort,
            "limit": limit,
            "offset": offset,
        ]
        if let subPath, !subPath.isEmpty { args["subPath"] = subPath }
        if let query, !query.isEmpty { args["query"] = query }
        return try await command("browse_collection_folder", arguments: args)
    }

    // MARK: 写真图集 V2

    func listGalleryModels() async throws -> [GalleryModelStat] {
        try await command("list_gallery_models")
    }

    func listGallerySets(
        modelName: String? = nil,
        query: String? = nil,
        sort: String = "count",
        limit: Int = 80,
        offset: Int = 0
    ) async throws -> GallerySetsPage {
        var args: [String: Any] = [
            "sort": sort,
            "limit": limit,
            "offset": offset,
        ]
        if let modelName, !modelName.isEmpty { args["modelName"] = modelName }
        if let query, !query.isEmpty { args["query"] = query }
        return try await command("list_gallery_sets", arguments: args)
    }

    func getGallerySetItems(setId: String) async throws -> [CreatorAssetV2] {
        try await command("get_gallery_set_items", arguments: ["setId": setId])
    }
}
