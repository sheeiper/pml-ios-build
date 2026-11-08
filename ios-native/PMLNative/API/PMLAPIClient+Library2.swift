import Foundation

// MARK: - M4 / M5 命令

extension PMLAPIClient {
    func libraryOverview() async throws -> LibraryOverview {
        try await command("get_library_overview")
    }

    func libraryAuthors(categoryId: String? = nil, filter: String = "all") async throws -> [LibraryAuthor] {
        var arguments: [String: Any] = ["filter": filter]
        if let categoryId { arguments["categoryId"] = categoryId }
        return try await command("list_library_authors", arguments: arguments)
    }

    func playbackHistory(limit: Int = 100) async throws -> [PlaybackHistoryItem] {
        try await command("list_playback_history", arguments: ["limit": limit])
    }

    func setPlaybackProgress(assetId: String, seconds: Double) async throws {
        try await perform(
            "set_asset_playback_progress",
            arguments: ["assetId": assetId, "positionSeconds": seconds]
        )
    }

    func clearPlaybackHistory() async throws {
        try await perform("clear_playback_history")
    }

    func favoriteCount() async throws -> Int64 {
        try await command("get_favorite_count")
    }

    func startScan(rootId: String? = nil, folderPath: String? = nil) async throws {
        var arguments: [String: Any] = [:]
        if let rootId { arguments["rootId"] = rootId }
        if let folderPath { arguments["folderPath"] = folderPath }
        try await perform("start_scan", arguments: arguments)
    }

    func mediaStreamInfo(assetId: String) async throws -> MediaStreamInfo {
        try await command("get_media_stream_info", arguments: ["assetId": assetId])
    }

    func scanProgress() async throws -> ScanProgress {
        try await command("get_scan_progress")
    }

    func cacheStats() async throws -> CacheStats {
        try await command("get_cache_stats")
    }

    func clearCache(target: String) async throws {
        try await perform("clear_cache", arguments: ["target": target])
    }

    func automationSettings() async throws -> AutomationSettings {
        try await command("get_automation_settings")
    }

    func saveAutomationSettings(_ settings: AutomationSettings) async throws {
        let data = try JSONEncoder().encode(settings)
        let object = try JSONSerialization.jsonObject(with: data)
        try await perform("set_automation_settings", arguments: ["settings": object])
    }

    func alistAccelConfig() async throws -> AlistAccelConfig {
        try await command("get_alist_accel_config")
    }

    func saveAlistAccelConfig(_ config: AlistAccelConfig) async throws {
        let data = try JSONEncoder().encode(config)
        let object = try JSONSerialization.jsonObject(with: data)
        try await perform("set_alist_accel_config", arguments: ["config": object])
    }

    func testAlistAccel(_ config: AlistAccelConfig) async throws -> AlistTestResult {
        let data = try JSONEncoder().encode(config)
        let object = try JSONSerialization.jsonObject(with: data)
        return try await command("test_alist_accel_connection", arguments: ["config": object])
    }
}
