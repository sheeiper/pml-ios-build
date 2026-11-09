import Foundation

struct DomainSummary: Decodable, Identifiable {
    let domain: String
    let rootId: String?
    let rootPath: String?
    let total: Int64
    let present: Int64
    let missing: Int64
    let lastScanAt: Int64?
    let lastScanStatus: String?

    var id: String { domain }
    var title: String {
        switch domain {
        case "recent": return "最新添加"
        case "pending": return "待整理"
        case "vault": return "已整理"
        case "comics": return "漫画"
        default: return domain
        }
    }
    var symbol: String {
        switch domain {
        case "recent": return "clock.arrow.circlepath"
        case "pending": return "tray.and.arrow.down.fill"
        case "vault": return "folder.fill"
        case "comics": return "book.closed.fill"
        default: return "square.grid.2x2"
        }
    }
}

struct DomainAsset: Decodable, Identifiable, Hashable {
    let id: String
    let domain: String
    let remotePath: String
    let fileName: String
    let parentPath: String
    let mediaType: String
    let sizeBytes: Int64
    let missing: Bool
    let modifiedAt: String?
    let thumbnailAssetId: String?
    let durationSeconds: Int64?
    let progressSeconds: Double?
    let favorite: Bool

    var isVideo: Bool { mediaType == "video" }
    var isImage: Bool { mediaType == "image" || mediaType == "comicPage" }
    var isComic: Bool { mediaType == "comicArchive" || mediaType == "pdf" }
    var symbol: String {
        if isVideo { return "play.rectangle.fill" }
        if isComic { return "book.closed.fill" }
        if isImage { return "photo.fill" }
        return "doc.fill"
    }
}

struct PendingProject: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
    let sourcePath: String
    let status: String
    let isLoose: Bool
    let totalCount: Int64
    let triageCount: Int64
    let readyCount: Int64
    let conflictCount: Int64
}

struct PendingCandidate: Decodable, Hashable {
    let id: String
    let name: String
}

struct PendingDomainItem: Decodable, Identifiable, Hashable {
    let assetId: String
    let remotePath: String
    let fileName: String
    let parentPath: String
    let mediaType: String
    let sizeBytes: Int64
    let recommendedCategoryId: String?
    let recommendedCategoryName: String?
    let recommendedAuthorId: String?
    let recommendedAuthorName: String?
    let candidateAuthors: [PendingCandidate]
    let candidateCategories: [PendingCandidate]
    let confidence: Double?
    let reason: String?
    let manualCategoryId: String?
    let manualAuthorId: String?
    let assignmentStatus: String
    let conflictReason: String?

    var id: String { assetId }
    var isReady: Bool { assignmentStatus == "ready" }
}

struct DomainChild: Decodable, Identifiable, Hashable {
    let path: String
    let name: String
    let isDirectory: Bool
    let assetId: String?
    let mediaType: String?
    let sizeBytes: Int64
    let modifiedAt: String?
    let thumbnailAssetId: String?
    let childCount: Int64

    var id: String { path }
    var isVideo: Bool { mediaType == "video" }
    var isImage: Bool { mediaType == "image" || mediaType == "comicPage" }
    var isComic: Bool { mediaType == "comicArchive" || mediaType == "pdf" }
}

struct DomainMetaAuthor: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
}

struct DomainMetaCategory: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
    let parentId: String?
}

struct DomainMeta: Decodable {
    let authors: [DomainMetaAuthor]
    let categories: [DomainMetaCategory]
}

struct DomainScanResult: Decodable {
    let domain: String
    let scanId: String
    let scanned: Int64
    let added: Int64
    let updated: Int64
    let missing: Int64
    let deleted: Int64
    let conflicts: Int64
}

struct DomainCleanResult: Decodable {
    let domain: String
    let dryRun: Bool
    let count: Int64
}

struct PendingMoveResult: Decodable {
    let success: Bool
    let status: String
    let message: String
    let sourcePath: String
    let targetPath: String
}

struct MoveConflict: Decodable, Identifiable {
    let id: String
    let assetId: String
    let projectId: String?
    let sourcePath: String
    let targetPath: String
    let kind: String
    let detectedAt: Int64
    let resolvedAt: Int64?
}

extension PMLAPIClient {
    func domainSummaries() async throws -> [DomainSummary] {
        try await command("four_domain_summaries")
    }

    func scanDomain(_ domain: String) async throws -> DomainScanResult {
        try await command("scan_domain", arguments: ["domain": domain])
    }

    func cleanDomain(_ domain: String, dryRun: Bool = true) async throws -> DomainCleanResult {
        try await command("clean_domain", arguments: ["domain": domain, "dryRun": dryRun])
    }

    func domainAssets(
        domain: String,
        mediaType: String? = nil,
        query: String? = nil,
        limit: Int = 200,
        offset: Int = 0
    ) async throws -> [DomainAsset] {
        var arguments: [String: Any] = ["domain": domain, "limit": limit, "offset": offset]
        if let mediaType, !mediaType.isEmpty { arguments["mediaType"] = mediaType }
        if let query, !query.isEmpty { arguments["query"] = query }
        return try await command("list_domain_assets", arguments: arguments)
    }


    func domainChildren(domain: String, path: String? = nil) async throws -> [DomainChild] {
        var arguments: [String: Any] = ["domain": domain]
        if let path, !path.isEmpty { arguments["path"] = path }
        return try await command("list_domain_children", arguments: arguments)
    }

    func pendingProjects() async throws -> [PendingProject] {
        try await command("list_pending_projects")
    }

    func pendingProjectItems(projectId: String) async throws -> [PendingDomainItem] {
        try await command("pending_project_items", arguments: ["projectId": projectId])
    }

    func domainMeta() async throws -> DomainMeta {
        try await command("list_domain_meta")
    }

    func assignPending(assetId: String, categoryId: String?, authorId: String?) async throws {
        var arguments: [String: Any] = ["assetId": assetId]
        if let categoryId { arguments["categoryId"] = categoryId }
        if let authorId { arguments["authorId"] = authorId }
        try await perform("assign_pending_item", arguments: arguments)
    }

    func movePendingDomainItem(assetId: String) async throws -> PendingMoveResult {
        try await command("move_pending_item", arguments: ["assetId": assetId])
    }

    func createPendingProject(name: String, assetIds: [String]) async throws -> PendingProject {
        try await command("create_pending_project", arguments: ["name": name, "assetIds": assetIds])
    }

    func moveConflicts() async throws -> [MoveConflict] {
        try await command("list_move_conflicts")
    }
}
