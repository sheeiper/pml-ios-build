import Foundation

// MARK: - V2 多模块配置与扫描

struct LibraryModuleInfo: Decodable, Identifiable, Hashable {
    let id: String
    let kind: String
    let name: String
    let remotePath: String
    let domain: String
    let enabled: Bool
    let sortOrder: Int64
    let lastScannedAt: Int64?
    let assetCount: Int64
    let totalBytes: Int64
    let entityCount: Int64

    var kindLabel: String {
        switch kind {
        case "movies": return "影视库"
        case "creators": return "创作者"
        case "collections": return "专题合集"
        case "galleries": return "写真图集"
        case "comics": return "漫画库"
        default: return kind
        }
    }

    var entityUnitLabel: String {
        switch kind {
        case "movies": return "部影片"
        case "creators": return "位创作者"
        case "collections": return "个合集"
        case "galleries": return "套图集"
        case "comics": return "本漫画"
        default: return "项"
        }
    }

    var symbol: String {
        switch kind {
        case "movies": return "film.stack.fill"
        case "creators": return "person.2.fill"
        case "collections": return "square.stack.3d.up.fill"
        case "galleries": return "photo.on.rectangle.angled"
        case "comics": return "book.closed.fill"
        default: return "folder.fill"
        }
    }
}

struct ModuleRebuildSummary: Decodable {
    let moduleId: String
    let kind: String
    let taggedAssets: Int64
    let entityCount: Int64
    let itemCount: Int64
}

struct DomainScanSummaryV2: Decodable {
    let domain: String
    let scanId: String
    let scanned: Int64
    let added: Int64
    let updated: Int64
    let missing: Int64
    let deleted: Int64
    let conflicts: Int64
}

struct ModuleScanResult: Decodable {
    let moduleId: String
    let scan: DomainScanSummaryV2
    let rebuild: ModuleRebuildSummary
}

// MARK: - V2 创作者与文件夹层级浏览

struct CreatorStudioStat: Decodable, Identifiable, Hashable {
    let studio: String
    let creatorCount: Int64
    let assetCount: Int64
    let totalBytes: Int64

    var id: String { studio }
}

struct CreatorSummaryV2: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
    let aliases: [String]
    let avatarAssetId: String?
    let bannerAssetId: String?
    let coverAssetId: String?
    let bio: String?
    let socialLinks: [String: String]
    let studios: [String]
    let studioCounts: [String: Int64]
    let videoCount: Int64
    let imageCount: Int64
    let albumCount: Int64
    let mediaCount: Int64
    let totalBytes: Int64
    let latestUpdatedAt: Int64

    var primaryCoverId: String? {
        avatarAssetId ?? coverAssetId ?? bannerAssetId
    }
}

struct CreatorAssetV2: Decodable, Identifiable, Hashable {
    let id: String
    let rootId: String
    let rootKind: String
    let remotePath: String
    let fileName: String
    let parentPath: String
    let mediaType: String
    let sizeBytes: Int64
    let durationSeconds: Int64?
    let authorId: String?
    let authorName: String?
    let categoryName: String?
    let thumbnailAssetId: String?
    let albumName: String?

    var isVideo: Bool {
        if mediaType == "video" { return true }
        let ext = (fileName as NSString).pathExtension.lowercased()
        return ["mp4", "mkv", "mov", "avi", "wmv", "flv", "webm", "m4v", "ts"].contains(ext)
    }

    var isImage: Bool {
        if mediaType == "image" || mediaType == "comicPage" { return true }
        let ext = (fileName as NSString).pathExtension.lowercased()
        return ["jpg", "jpeg", "png", "webp", "gif", "heic", "bmp"].contains(ext)
    }

    var isComicArchive: Bool {
        if mediaType == "comicArchive" || mediaType == "pdf" { return true }
        let ext = (fileName as NSString).pathExtension.lowercased()
        return ["zip", "cbz", "rar", "cbr", "7z", "pdf"].contains(ext)
    }

    var effectiveThumbId: String {
        thumbnailAssetId ?? id
    }

    func toRemoteBrowserEntry() -> RemoteBrowserEntry {
        RemoteBrowserEntry(
            path: remotePath,
            name: fileName,
            isDirectory: false,
            sizeBytes: sizeBytes,
            modifiedAt: nil,
            etag: nil,
            assetId: id,
            mediaType: mediaType,
            thumbnailSource: nil,
            coverAssetId: thumbnailAssetId ?? id,
            durationSeconds: durationSeconds,
            progressSeconds: nil
        )
    }
}

struct FolderCardV2: Decodable, Identifiable, Hashable {
    let name: String
    let subPath: String
    let studioName: String
    let itemCount: Int64
    let videoCount: Int64
    let imageCount: Int64
    let totalBytes: Int64
    let coverAssetId: String?
    let previewAssetIds: [String]

    var id: String { "\(studioName)|\(subPath)" }

    /// 文件夹里仅包着 1 个视频（无其他文件）时，支持一键直播免进空壳目录。
    var isSingleVideoWrapper: Bool {
        itemCount == 1 && videoCount == 1 && imageCount == 0
    }

    var singleVideoAssetId: String? {
        guard isSingleVideoWrapper else { return nil }
        return previewAssetIds.first ?? coverAssetId
    }
}

struct FolderBrowsePage: Decodable {
    let subPath: String
    let breadcrumbs: [String]
    let folders: [FolderCardV2]
    let files: [CreatorAssetV2]
    let totalFolders: Int64
    let totalFiles: Int64
    let total: Int64
}

struct CreatorFolderResponse: Decodable {
    let creator: CreatorSummaryV2
    let page: FolderBrowsePage
}

// MARK: - V2 创作者建议合并

struct MergeCandidateAuthor: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
    let aliases: [String]
    let studios: [String]
    let paths: [String]
    let mediaCount: Int64
    let totalBytes: Int64
    let coverAssetId: String?
}

struct CreatorMergeSuggestion: Decodable, Identifiable, Hashable {
    let authorA: MergeCandidateAuthor
    let authorB: MergeCandidateAuthor
    let reason: String
    let score: Int64

    var id: String { "\(authorA.id)|\(authorB.id)" }
}

struct VerifyCreatorMergeResult: Decodable {
    let keepAuthorId: String
    let keepAuthorName: String
    let mergedAuthorId: String
    let mergedAuthorName: String
    let addedAliases: [String]
    let remainingOldAssets: Int64
    let keepTotalAssets: Int64
    let fullyMerged: Bool
}

// MARK: - V2 专题合集

struct CollectionChannelStat: Decodable, Identifiable, Hashable {
    let channel: String
    let albumCount: Int64
    let itemCount: Int64
    let videoCount: Int64
    let totalBytes: Int64

    var id: String { channel }
}

struct CollectionAlbumV2: Decodable, Identifiable, Hashable {
    let id: String
    let channel: String
    let name: String
    let remotePath: String
    let isDirect: Bool
    let coverAssetId: String?
    let previewAssetIds: [String]
    let itemCount: Int64
    let videoCount: Int64
    let imageCount: Int64
    let totalBytes: Int64
    let updatedAt: Int64

    /// 单视频外壳合集支持一键直播。
    var isSingleVideoWrapper: Bool {
        itemCount == 1 && videoCount == 1 && imageCount == 0
    }

    var singleVideoAssetId: String? {
        guard isSingleVideoWrapper else { return nil }
        return previewAssetIds.first ?? coverAssetId
    }
}

struct CollectionAlbumsPage: Decodable {
    let albums: [CollectionAlbumV2]
    let total: Int64
}

struct CollectionFolderResponse: Decodable {
    let album: CollectionAlbumV2
    let page: FolderBrowsePage
}

// MARK: - V2 写真图集

struct GalleryModelStat: Decodable, Identifiable, Hashable {
    let modelName: String
    let setCount: Int64
    let itemCount: Int64
    let totalBytes: Int64

    var id: String { modelName }
}

struct GallerySetV2: Decodable, Identifiable, Hashable {
    let id: String
    let modelName: String
    let albumName: String
    let remotePath: String
    let isArchive: Bool
    let coverAssetId: String?
    let itemCount: Int64
    let totalBytes: Int64
    let updatedAt: Int64
}

struct GallerySetsPage: Decodable {
    let sets: [GallerySetV2]
    let total: Int64
}

// MARK: - 通用格式化工具

enum PMLFormatters {
    static func bytes(_ value: Int64) -> String {
        guard value > 0 else { return "0 B" }
        let units = ["B", "KB", "MB", "GB", "TB"]
        var v = Double(value)
        var idx = 0
        while v >= 1024 && idx < units.count - 1 {
            v /= 1024
            idx += 1
        }
        return v >= 100 ? String(format: "%.0f %@", v, units[idx]) : String(format: "%.1f %@", v, units[idx])
    }

    static func duration(_ seconds: Int64?) -> String? {
        guard let seconds, seconds > 0 else { return nil }
        let h = seconds / 3600
        let m = (seconds % 3600) / 60
        let s = seconds % 60
        if h > 0 {
            return String(format: "%d:%02d:%02d", h, m, s)
        }
        return String(format: "%02d:%02d", m, s)
    }
}
