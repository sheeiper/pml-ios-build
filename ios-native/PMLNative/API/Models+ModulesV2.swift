import Foundation

// MARK: - V2 多模块配置与扫描

struct LibraryModuleInfo: Decodable, Identifiable, Hashable {
    let id: String
    let kind: String
    let name: String
    let remotePath: String
    let remotePaths: [String]?
    let domain: String
    let enabled: Bool
    let sortOrder: Int64
    let lastScannedAt: Int64?
    let assetCount: Int64
    let totalBytes: Int64
    let entityCount: Int64

    var allPaths: [String] {
        if let list = remotePaths, !list.isEmpty { return list }
        return remotePath.isEmpty ? [] : [remotePath]
    }

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
    let coverPageIndex: Int?
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


// MARK: - 漫画库 V1（脱离目录自动识别作者与系列）

struct ComicWorkItem: Decodable, Identifiable, Hashable {
    let id: String
    let title: String
    let authors: [String]
    let circle: String?
    let parody: String?
    let eventTag: String?
    let translators: [String]
    let tags: [String]
    let language: String
    let isUncensored: Bool
    let isFullColor: Bool
    let isDl: Bool
    let isOngoing: Bool
    let coverAssetId: String?
    let coverIsArchive: Bool
    let coverPageIndex: Int?
    let volumeCount: Int64
    let totalPages: Int64?
    let totalBytes: Int64
    let source: String
    let category: String?
    let publishedAt: String?
    let rating: Double?
    let summary: String?
    let updatedAt: Int64
    let lastReadVolumeId: String?
    let lastReadVolumeLabel: String?
    let readPage: Int64?
    let readTotalPages: Int64?
    let readPercent: Int64?
    let readStatus: String?
    let lastReadAt: Int64?
}

struct ComicVolumeItem: Decodable, Identifiable, Hashable {
    let id: String
    let workId: String
    let volumeLabel: String
    let sortIndex: Double
    let isArchive: Bool
    let assetId: String
    let folderPath: String?
    let fileName: String
    let remotePath: String
    let pageCount: Int64
    let sizeBytes: Int64
    let readPage: Int64?
    let completed: Bool?
}

struct ComicWorkDetail: Decodable {
    let work: ComicWorkItem
    let volumes: [ComicVolumeItem]
    let relatedByAuthor: [ComicWorkItem]
    let relatedByParody: [ComicWorkItem]
}

struct ComicLibraryPage: Decodable {
    let items: [ComicWorkItem]
    let total: Int64
    let hasMore: Bool
}

struct ComicAuthorItem: Decodable, Identifiable, Hashable {
    var id: String { name }
    let name: String
    let circle: String?
    let workCount: Int64
    let volumeCount: Int64
    let coverAssetId: String?
    let coverIsArchive: Bool
}

struct ComicFilterFacet: Decodable, Identifiable, Hashable {
    var id: String { name }
    let name: String
    let count: Int64
}

struct ComicFilterOptions: Decodable {
    let categories: [ComicFilterFacet]?
    let genres: [ComicFilterFacet]?
    let years: [ComicFilterFacet]?
    let authors: [ComicFilterFacet]
    let parodies: [ComicFilterFacet]
    let tags: [ComicFilterFacet]
}

struct ComicLibraryStats: Decodable {
    let totalWorks: Int64
    let totalVolumes: Int64
    let totalSeries: Int64
    let totalAuthors: Int64
    let zhCount: Int64
    let rawCount: Int64
    let uncensoredCount: Int64
    let archiveVolumes: Int64
    let folderVolumes: Int64
    let enrichedCount: Int64?
}

struct ComicExternalCandidate: Codable, Identifiable {
    var id: String { "\(source):\(externalId)" }
    let source: String
    let externalId: String
    let externalUrl: String?
    let title: String
    let authors: [String]
    let circle: String?
    let parody: String?
    let tags: [String]
    let rating: Double?
    let publishedAt: String?
    let summary: String?
    let coverUrl: String?
    let matchReason: String
}

struct ComicBatchResultItem: Decodable, Identifiable {
    var id: String { "\(workId)-\(ok)" }
    let workId: String
    let title: String
    let ok: Bool
    let source: String?
    let tagsAdded: Int
    let error: String?
}

struct ComicBatchProgress: Decodable {
    let running: Bool
    let total: Int
    let completed: Int
    let succeeded: Int
    let failed: Int
    let currentTitle: String?
    let lastError: String?
    let recentItems: [ComicBatchResultItem]?
}

struct OnlineComicConfigView: Decodable {
    let jmDomains: [String]
    let jmImageDomains: [String]
    let picaEmail: String
    let picaHasPassword: Bool
    let picaLoggedIn: Bool
    let picaChannel: String
}

struct OnlineComicItem: Decodable, Identifiable, Hashable {
    var id: String { "\(source):\(comicId)" }
    let source: String
    let comicId: String
    let title: String
    let author: String
    let coverUrl: String
    let category: String
    let tags: [String]

    private enum CodingKeys: String, CodingKey {
        case source
        case comicId = "id"
        case title, author, coverUrl, category, tags
    }
}

struct OnlineComicPageResult: Decodable {
    let source: String
    let items: [OnlineComicItem]
    let page: Int
    let hasMore: Bool
    let total: Int64?
}

struct OnlineComicChapter: Decodable, Identifiable, Hashable {
    let id: String
    let order: Int
    let title: String
}

struct OnlineComicDetail: Decodable {
    let source: String
    let id: String
    let title: String
    let authors: [String]
    let coverUrl: String
    let description: String
    let tags: [String]
    let isFavourite: Bool?
    let chapters: [OnlineComicChapter]
}

struct OnlineChapterPagesResult: Decodable {
    let source: String
    let comicId: String
    let chapterId: String
    let images: [String]
}

struct ComicMagnetItem: Decodable, Identifiable, Hashable {
    var id: String { infoHash }
    let title: String
    let magnet: String
    let infoHash: String
    let size: String
    let seeders: Int
    let leechers: Int
    let pubDate: String
    let source: String
}

struct CbzPackageTask: Decodable {
    let taskId: String
    let status: String
    let title: String
    let total: Int
    let completed: Int
    let fileName: String?
    let downloadUrl: String?
    let sizeBytes: UInt64?
    let error: String?
}

struct UniversalMagnetItem: Decodable, Identifiable, Hashable {
    var id: String { infoHash }
    let title: String
    let magnet: String
    let infoHash: String
    let sizeBytes: UInt64
    let sizeText: String
    let seeders: Int
    let leechers: Int
    let category: String
    let source: String
    let tracker: String
    let pubDate: String
    let timestamp: Int64
    let cnSub: Bool?
    let hd: Bool?
}

struct MagnetSourceStat: Decodable, Identifiable, Hashable {
    var id: String { source }
    let source: String
    let label: String
    let count: Int
    let ok: Bool
    let elapsedMs: UInt64
    let error: String?
}

struct MagnetSearchResponse: Decodable {
    let query: String
    let total: Int
    let items: [UniversalMagnetItem]
    let sourceStats: [MagnetSourceStat]
}

struct MagnetFileEntry: Decodable, Identifiable, Hashable {
    var id: String { "\(path)-\(sizeBytes)" }
    let path: String
    let sizeBytes: UInt64
    let sizeText: String
}

struct MagnetFilesResponse: Decodable {
    let infoHash: String
    let name: String
    let totalSizeBytes: UInt64
    let totalSizeText: String
    let fileCount: Int
    let files: [MagnetFileEntry]
    let source: String
}

struct OfflineDownloadConfig: Decodable {
    let inboxPath: String
    let trashPath: String
    let movieDir: String
    let comicDir: String
    let galleryDir: String
    let creatorPendingDir: String?
    let galleryPendingDir: String?
    let comicPendingDir: String?
    let collectionPendingDir: String?
}

struct OfflineDownloadTask: Decodable, Identifiable, Hashable {
    let id: String
    let infoHash: String
    let title: String
    let magnet: String
    let targetDir: String
    let category: String
    let status: String
    let message: String?
    let createdAt: Int64
}

struct OfflineQuotaInfo: Decodable {
    let total: Int64
    let used: Int64
    let left: Int64
}

struct CloudOfflineFile: Decodable, Identifiable, Hashable {
    var id: String { "\(infoHash)-\(fileId)-\(addTime)" }
    let name: String
    let sizeBytes: UInt64
    let sizeText: String
    let url: String
    let status: Int
    let statusCategory: String
    let infoHash: String
    let fileId: String
    let addTime: Int64
    let percentDone: Double
}

struct OfflineCloudStatusResponse: Decodable {
    let cloudName: String
    let quota: OfflineQuotaInfo
    let page: Int
    let pageSize: Int
    let pageCount: Int
    let totalCount: Int64
    let items: [CloudOfflineFile]
}

struct BatchOfflineSubmitItemResult: Decodable, Identifiable {
    var id: String { "\(input)-\(ok)" }
    let input: String
    let ok: Bool
    let message: String
}

struct BatchOfflineSubmitResponse: Decodable {
    let total: Int
    let succeeded: Int
    let failed: Int
    let targetDir: String
    let results: [BatchOfflineSubmitItemResult]
}

struct OfflinedSignaturesResponse: Decodable {
    let hashes: [String]
    let urls: [String]
    let cloudSyncedAt: Int64?
}

struct InboxFileEntry: Decodable, Identifiable, Hashable {
    var id: String { fullPath }
    let name: String
    let fullPath: String
    let sizeBytes: UInt64
    let sizeText: String
    let reason: String
}

struct InboxFileRename: Codable, Identifiable, Hashable {
    var id: String { fullPath }
    let fullPath: String
    let oldName: String
    let newName: String
}

struct InboxPlanItem: Decodable, Identifiable, Hashable {
    var id: String { fullPath }
    var name: String
    var fullPath: String
    let isDir: Bool
    let totalSizeBytes: UInt64
    let totalSizeText: String
    let suggestedName: String
    let suggestedCategory: String
    var needsClean: Bool
    let flattenSourceDir: String?
    let flattenWrapperDir: String?
    let flattenLevels: Int?
    let keptFiles: [InboxFileEntry]
    var junkFiles: [InboxFileEntry]
    var fileRenames: [InboxFileRename]?
}

struct InboxScanResponse: Decodable {
    let inboxPath: String
    let trashPath: String
    let config: OfflineDownloadConfig
    let items: [InboxPlanItem]
}

struct CleanInboxItemResponse: Decodable {
    let ok: Bool
    let movedTrashCount: Int
    let newPath: String
    let message: String
}

struct ArchiveInboxItemResponse: Decodable {
    let ok: Bool
    let destDir: String
    let message: String
}

struct InboxTrashItem: Decodable, Identifiable, Hashable {
    var id: String { fullPath }
    let name: String
    let fullPath: String
    let isDir: Bool
    let sizeBytes: UInt64
    let sizeText: String
}

struct InboxTrashGroup: Decodable, Identifiable, Hashable {
    var id: String { groupPath }
    let groupName: String
    let groupPath: String
    let suggestedRestoreDir: String
    let restoreDirExists: Bool
    let items: [InboxTrashItem]
}

struct InboxTrashResponse: Decodable {
    let trashPath: String
    let inboxPath: String
    let groups: [InboxTrashGroup]
    let looseItems: [InboxTrashItem]
}

struct RestoreInboxTrashResponse: Decodable {
    let ok: Bool
    let restoredCount: Int
    let targetDir: String
    let message: String
}

struct Cd2FolderEntry: Decodable, Identifiable, Hashable {
    var id: String { fullPath }
    let name: String
    let fullPath: String
}

struct Cd2FoldersResponse: Decodable {
    let currentPath: String
    let parentPath: String?
    let folders: [Cd2FolderEntry]
}
