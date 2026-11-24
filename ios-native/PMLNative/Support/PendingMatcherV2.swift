import Foundation

/// 五大模块枚举（用于待收纳整理 2 级内联快捷选区与智能匹配）。
enum PendingModuleKind: String, CaseIterable, Identifiable {
    case movies
    case creators
    case collections
    case galleries
    case comics

    var id: String { rawValue }

    var label: String {
        switch self {
        case .movies: return "影视库"
        case .creators: return "创作者"
        case .collections: return "专题合集"
        case .galleries: return "写真图集"
        case .comics: return "漫画"
        }
    }

    var symbol: String {
        switch self {
        case .movies: return "film.stack.fill"
        case .creators: return "person.2.fill"
        case .collections: return "square.stack.3d.up.fill"
        case .galleries: return "photo.on.rectangle.angled"
        case .comics: return "book.closed.fill"
        }
    }

    /// 创作者/图集新建子目录时是否自动套【】书名号
    var usesBracketsForCreator: Bool {
        self == .creators
    }
}

struct PendingModuleSubOption: Identifiable, Hashable {
    let id: String
    let label: String
    let path: String
}

struct PendingV2Context {
    let modules: [LibraryModuleInfo]
    let studios: [CreatorStudioStat]
    let creators: [CreatorSummaryV2]
    let channels: [CollectionChannelStat]
    let galleryModels: [GalleryModelStat]

    static let empty = PendingV2Context(
        modules: [],
        studios: [],
        creators: [],
        channels: [],
        galleryModels: []
    )

    var isLoaded: Bool { !modules.isEmpty }

    func rootPath(for kind: PendingModuleKind, fallbackVaultRoot: String) -> String {
        if let mod = modules.first(where: { $0.kind == kind.rawValue && !$0.remotePath.isEmpty }) {
            return mod.remotePath.hasSuffix("/") ? String(mod.remotePath.dropLast()) : mod.remotePath
        }
        let base = fallbackVaultRoot.hasSuffix("/") ? String(fallbackVaultRoot.dropLast()) : fallbackVaultRoot
        guard !base.isEmpty else { return "" }
        switch kind {
        case .movies: return "\(base)/媒体库"
        case .creators: return "\(base)/创作者"
        case .collections: return "\(base)/合集"
        case .galleries: return "\(base)/图集"
        case .comics: return "\(base)/漫画"
        }
    }

    func subOptions(for kind: PendingModuleKind, fallbackVaultRoot: String) -> [PendingModuleSubOption] {
        let root = rootPath(for: kind, fallbackVaultRoot: fallbackVaultRoot)
        guard !root.isEmpty else { return [] }
        switch kind {
        case .movies:
            let subs = ["日韩番号", "FC2", "欧美", "动漫", "国产剧情"]
            return subs.map { PendingModuleSubOption(id: "\(root)/\($0)", label: $0, path: "\(root)/\($0)") }
        case .creators:
            let names = studios.map(\.studio)
            return names.map { PendingModuleSubOption(id: "\(root)/\($0)", label: $0, path: "\(root)/\($0)") }
        case .collections:
            let names = channels.map(\.channel)
            return names.map { PendingModuleSubOption(id: "\(root)/\($0)", label: $0, path: "\(root)/\($0)") }
        case .galleries:
            let names = galleryModels.map(\.modelName)
            return names.map { PendingModuleSubOption(id: "\(root)/\($0)", label: $0, path: "\(root)/\($0)") }
        case .comics:
            let subs = ["无修正", "单行本", "同人志", "全彩", "短篇"]
            return subs.map { PendingModuleSubOption(id: "\(root)/\($0)", label: $0, path: "\(root)/\($0)") }
        }
    }
}

struct PendingMatchV2Result {
    let module: PendingModuleKind
    let targetPath: String
    let categoryPath: String
    let categoryName: String
    let authorId: String?
    let authorName: String?
    let matchedAlias: String?
    let source: PendingAssignmentSource
}

enum PendingMatcherV2 {
    private static let fc2Regex = try? NSRegularExpression(pattern: #"(?i)\bFC2[-_\s]?PPV[-_\s]?\d{5,7}\b"#)
    private static let avCodeRegex = try? NSRegularExpression(pattern: #"(?i)\b[A-Z]{2,6}-\d{2,5}\b"#)

    /// 将 V2 创作者列表转换成旧工作台兼容的 VaultAuthorEntry，以便联邦作者池与同作者传播无缝工作。
    static func buildAuthorEntries(from ctx: PendingV2Context, fallbackVaultRoot: String) -> [VaultAuthorEntry] {
        let creatorsRoot = ctx.rootPath(for: .creators, fallbackVaultRoot: fallbackVaultRoot)
        return ctx.creators.map { creator in
            let primaryStudio = creator.studios.first ?? ""
            let catPath: String? = {
                guard !creatorsRoot.isEmpty else { return nil }
                return primaryStudio.isEmpty ? creatorsRoot : "\(creatorsRoot)/\(primaryStudio)"
            }()
            let vaultPath: String? = {
                guard let catPath else { return nil }
                return "\(catPath)/【\(creator.name)】"
            }()
            return VaultAuthorEntry(
                id: creator.id,
                canonicalName: creator.name,
                vaultPath: vaultPath,
                categoryPath: catPath,
                aliases: creator.aliases
            )
        }
    }

    /// 将五大模块及其二级子目录转换为 VaultCategoryNode 树，供高级分类选择器与最近分类复用。
    static func buildCategoryTree(from ctx: PendingV2Context, fallbackVaultRoot: String) -> [VaultCategoryNode] {
        var nodes: [VaultCategoryNode] = []
        for kind in PendingModuleKind.allCases {
            let root = ctx.rootPath(for: kind, fallbackVaultRoot: fallbackVaultRoot)
            guard !root.isEmpty else { continue }
            let subs = ctx.subOptions(for: kind, fallbackVaultRoot: fallbackVaultRoot)
            let children = subs.map { sub in
                VaultCategoryNode(
                    id: sub.path,
                    rawName: sub.label,
                    cleanName: sub.label,
                    fullDisplayName: "\(kind.label) / \(sub.label)",
                    path: sub.path,
                    parentId: root,
                    children: []
                )
            }
            nodes.append(
                VaultCategoryNode(
                    id: root,
                    rawName: kind.label,
                    cleanName: kind.label,
                    fullDisplayName: kind.label,
                    path: root,
                    parentId: nil,
                    children: children
                )
            )
        }
        return nodes
    }

    /// 五大模块智能匹配引擎：番号/FC2 → 308 创作者+别名 → 15 写真模特 → 漫画 → 12 合集频道。
    static func match(
        fileName: String,
        isDirectory: Bool,
        context: PendingV2Context,
        fallbackVaultRoot: String
    ) -> PendingMatchV2Result? {
        let lower = fileName.lowercased()
        let ns = fileName as NSString
        let fullRange = NSRange(location: 0, length: ns.length)

        // 1. FC2 / AV 番号 → 影视库
        if let fc2Regex, fc2Regex.firstMatch(in: fileName, range: fullRange) != nil {
            let root = context.rootPath(for: .movies, fallbackVaultRoot: fallbackVaultRoot)
            if !root.isEmpty {
                let target = "\(root)/FC2"
                return PendingMatchV2Result(
                    module: .movies,
                    targetPath: target,
                    categoryPath: target,
                    categoryName: "影视库 / FC2",
                    authorId: nil,
                    authorName: nil,
                    matchedAlias: nil,
                    source: .matchedAuthor
                )
            }
        }
        if let avCodeRegex, avCodeRegex.firstMatch(in: fileName, range: fullRange) != nil {
            let root = context.rootPath(for: .movies, fallbackVaultRoot: fallbackVaultRoot)
            if !root.isEmpty {
                let target = "\(root)/日韩番号"
                return PendingMatchV2Result(
                    module: .movies,
                    targetPath: target,
                    categoryPath: target,
                    categoryName: "影视库 / 日韩番号",
                    authorId: nil,
                    authorName: nil,
                    matchedAlias: nil,
                    source: .matchedAuthor
                )
            }
        }

        // 2. 创作者库匹配（主名 + 别名）
        let creatorsRoot = context.rootPath(for: .creators, fallbackVaultRoot: fallbackVaultRoot)
        if !creatorsRoot.isEmpty {
            // 按名称长度降序，避免短名误伤长名
            let sortedCreators = context.creators.sorted { $0.name.count > $1.name.count }
            for creator in sortedCreators {
                let nameClean = creator.name.trimmingCharacters(in: .whitespacesAndNewlines)
                if nameClean.count >= 2 && lower.contains(nameClean.lowercased()) {
                    let studio = creator.studios.first ?? ""
                    let catPath = studio.isEmpty ? creatorsRoot : "\(creatorsRoot)/\(studio)"
                    let target = "\(catPath)/【\(creator.name)】"
                    return PendingMatchV2Result(
                        module: .creators,
                        targetPath: target,
                        categoryPath: catPath,
                        categoryName: studio.isEmpty ? "创作者" : "创作者 / \(studio)",
                        authorId: creator.id,
                        authorName: creator.name,
                        matchedAlias: nil,
                        source: .matchedAuthor
                    )
                }
                for alias in creator.aliases {
                    let aliasClean = alias.trimmingCharacters(in: .whitespacesAndNewlines)
                    if aliasClean.count >= 2 && lower.contains(aliasClean.lowercased()) {
                        let studio = creator.studios.first ?? ""
                        let catPath = studio.isEmpty ? creatorsRoot : "\(creatorsRoot)/\(studio)"
                        let target = "\(catPath)/【\(creator.name)】"
                        return PendingMatchV2Result(
                            module: .creators,
                            targetPath: target,
                            categoryPath: catPath,
                            categoryName: studio.isEmpty ? "创作者" : "创作者 / \(studio)",
                            authorId: creator.id,
                            authorName: creator.name,
                            matchedAlias: aliasClean,
                            source: .matchedAlias
                        )
                    }
                }
            }
        }

        // 3. 写真图集模特匹配
        let galleriesRoot = context.rootPath(for: .galleries, fallbackVaultRoot: fallbackVaultRoot)
        if !galleriesRoot.isEmpty {
            let sortedModels = context.galleryModels.sorted { $0.modelName.count > $1.modelName.count }
            for model in sortedModels where model.modelName.count >= 2 {
                if lower.contains(model.modelName.lowercased()) {
                    let target = "\(galleriesRoot)/\(model.modelName)"
                    return PendingMatchV2Result(
                        module: .galleries,
                        targetPath: target,
                        categoryPath: target,
                        categoryName: "写真图集 / \(model.modelName)",
                        authorId: nil,
                        authorName: nil,
                        matchedAlias: nil,
                        source: .matchedAuthor
                    )
                }
            }
        }

        // 4. 漫画特征匹配
        let comicsRoot = context.rootPath(for: .comics, fallbackVaultRoot: fallbackVaultRoot)
        if !comicsRoot.isEmpty {
            let isComicExt = lower.hasSuffix(".cbz") || lower.hasSuffix(".cbr")
            let hasComicKeyword = lower.contains("汉化") || lower.contains("同人志") || lower.contains("单行本") || lower.contains("漫画")
            if isComicExt || hasComicKeyword {
                let sub = lower.contains("同人") ? "同人志" : (lower.contains("全彩") ? "全彩" : "单行本")
                let target = "\(comicsRoot)/\(sub)"
                return PendingMatchV2Result(
                    module: .comics,
                    targetPath: target,
                    categoryPath: target,
                    categoryName: "漫画 / \(sub)",
                    authorId: nil,
                    authorName: nil,
                    matchedAlias: nil,
                    source: .matchedAlias
                )
            }
        }

        // 5. 专题合集频道匹配
        let collectionsRoot = context.rootPath(for: .collections, fallbackVaultRoot: fallbackVaultRoot)
        if !collectionsRoot.isEmpty {
            let sortedChannels = context.channels.sorted { $0.channel.count > $1.channel.count }
            for ch in sortedChannels where ch.channel.count >= 2 {
                if lower.contains(ch.channel.lowercased()) {
                    let target = "\(collectionsRoot)/\(ch.channel)"
                    return PendingMatchV2Result(
                        module: .collections,
                        targetPath: target,
                        categoryPath: target,
                        categoryName: "专题合集 / \(ch.channel)",
                        authorId: nil,
                        authorName: nil,
                        matchedAlias: nil,
                        source: .matchedAlias
                    )
                }
            }
        }

        _ = isDirectory
        return nil
    }
}
