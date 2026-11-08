import Foundation

// MARK: - 与 Rust 命令一一对应的便捷方法

extension PMLAPIClient {
    func libraryStatus() async throws -> LibraryStatus {
        try await command("get_library_status")
    }

    func libraryRoots() async throws -> [LibraryRoot] {
        try await command("list_library_roots")
    }

    func addLibraryRoot(kind: String, remotePath: String) async throws {
        try await perform("add_library_root", arguments: ["kind": kind, "remotePath": remotePath])
    }

    func removeLibraryRoot(rootId: String) async throws {
        try await perform("remove_library_root", arguments: ["rootId": rootId])
    }

    func remoteChildren(
        rootId: String,
        path: String,
        cursor: String? = nil,
        refresh: Bool = false,
        sort: String = "name"
    ) async throws -> RemoteBrowserPage {
        var arguments: [String: Any] = ["rootId": rootId, "path": path, "sort": sort]
        if let cursor { arguments["cursor"] = cursor }
        if refresh { arguments["refresh"] = true }
        return try await command("list_remote_children", arguments: arguments)
    }

    func assetThumbnail(assetId: String) async throws -> AssetThumbnail? {
        try await command("get_asset_thumbnail", arguments: ["assetId": assetId], as: AssetThumbnail?.self)
    }

    /// 返回本地代理的可播放绝对地址（Rust 端返回的是同源相对路径）。
    func mediaSourceURL(assetId: String) async throws -> URL {
        let raw: String = try await command("open_media_source", arguments: ["assetId": assetId])
        return try resolve(raw)
    }

    func providerConfig() async throws -> ProviderConfig {
        try await command("get_provider_config")
    }

    func saveProviderConfig(_ config: ProviderConfig) async throws {
        let data = try JSONEncoder().encode(config)
        let object = try JSONSerialization.jsonObject(with: data)
        try await perform("save_provider_config", arguments: ["config": object])
    }

    func restoreConnection() async throws -> LibraryStatus {
        try await command("restore_alist_connection")
    }

    func scanLibrary(rootId: String? = nil) async throws {
        var arguments: [String: Any] = [:]
        if let rootId { arguments["rootId"] = rootId }
        try await perform("scan_library", arguments: arguments)
    }

    // MARK: - 漫画

    func inspectComicArchive(assetId: String) async throws -> ComicArchiveInfo {
        try await command("inspect_comic_archive", arguments: ["assetId": assetId])
    }

    /// 复用网页版同一套 `/api/comic-pages/{asset}/{page}` 路由。
    func comicPageURL(assetId: String, pageIndex: Int) -> URL? {
        URL(string: "/api/comic-pages/\(assetId)/\(pageIndex)", relativeTo: configuration.baseURL)?.absoluteURL
    }

    // MARK: - 短视频

    func shortVideoFeed(cursor: String? = nil, limit: Int = 12, authorId: String? = nil, maxDurationSeconds: Int? = nil) async throws -> ShortVideoFeedPage {
        var arguments: [String: Any] = ["limit": limit]
        if let cursor { arguments["cursor"] = cursor }
        if let authorId, !authorId.isEmpty { arguments["authorId"] = authorId }
        if let maxDurationSeconds, maxDurationSeconds > 0 {
            arguments["maxDurationSeconds"] = maxDurationSeconds
        }
        return try await command("list_short_video_feed", arguments: arguments)
    }

    /// 把 `/media/token` 这类同源相对路径解析成本地代理的绝对地址。
    func resolve(_ path: String) throws -> URL {
        if let url = URL(string: path), url.scheme != nil {
            return url
        }
        guard let url = URL(string: path, relativeTo: configuration.baseURL) else {
            throw APIError.badURL
        }
        return url.absoluteURL
    }
}
