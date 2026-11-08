import Foundation

extension PMLAPIClient {
    // MARK: 封面 / 作者图片

    func uploadCustomCover(
        rootId: String?,
        targetType: String,
        targetIdOrPath: String,
        mimeType: String = "image/jpeg",
        dataBase64: String
    ) async throws -> String {
        var arguments: [String: Any] = [
            "targetType": targetType,
            "targetIdOrPath": targetIdOrPath,
            "mimeType": mimeType,
            "dataBase64": dataBase64,
        ]
        if let rootId { arguments["rootId"] = rootId }
        return try await command("upload_custom_cover", arguments: arguments)
    }

    func uploadAuthorAvatar(authorId: String, dataBase64: String) async throws {
        try await perform("upload_author_avatar", arguments: ["authorId": authorId, "dataBase64": dataBase64])
    }

    func uploadAuthorBanner(authorId: String, dataBase64: String) async throws {
        try await perform("upload_author_banner", arguments: ["authorId": authorId, "dataBase64": dataBase64])
    }

    // MARK: 重复文件

    func startDuplicateScan(rootId: String? = nil) async throws -> DuplicateScanProgress {
        try await command("start_duplicate_scan", arguments: ["rootId": rootId ?? NSNull()])
    }

    func duplicateScanProgress() async throws -> DuplicateScanProgress {
        try await command("get_duplicate_scan_progress")
    }

    func cancelDuplicateScan() async throws {
        try await perform("cancel_duplicate_scan")
    }

    func duplicateGroups(rootId: String? = nil) async throws -> [DuplicateGroup] {
        try await command("list_duplicate_groups", arguments: ["rootId": rootId ?? NSNull()])
    }

    // MARK: 扫描中心

    func rescanFailedDirectories() async throws -> ScanResult {
        try await command("rescan_failed_directories")
    }

    // MARK: Inbox 打包

    func packInbox(inboxPath: String, batchName: String, itemNames: [String]) async throws -> PackInboxResult {
        try await command(
            "pack_inbox_to_pending_batch",
            arguments: [
                "inboxPath": inboxPath,
                "batchName": batchName,
                "itemNames": itemNames,
            ]
        )
    }
}
