import Foundation

// MARK: - 待归纳 / 已归纳条目模型

enum PendingAssignmentSource: String, Hashable {
    case none
    case matchedAuthor
    case matchedAlias
    case manual
    case propagated
    case restoredDraft

    var badgeText: String {
        switch self {
        case .none: return "未指定"
        case .matchedAuthor: return "✨ 命中作者"
        case .matchedAlias: return "🏷️ 别名命中"
        case .manual: return "✋ 手动指定"
        case .propagated: return "🔗 同作者联动"
        case .restoredDraft: return "💾 已保存草稿"
        }
    }
}

struct PendingCurationItem: Identifiable, Hashable {
    let entry: RemoteBrowserEntry
    var assignmentSource: PendingAssignmentSource = .none
    var matchedAuthorId: String? = nil
    var matchedAuthor: String? = nil
    var matchedAlias: String? = nil
    var matchedCategoryPath: String? = nil
    var matchedCategoryName: String? = nil
    /// 最终物理目标目录（不含当前文件/文件夹名），如 `/115open/文件/{1-国产}/{1-原创}/{1-推特}/【小仙云】`
    /// 非空即属于「已归纳」，为空即属于「待归纳」
    var targetVaultPath: String? = nil
    /// 从文件名提取的候选作者词（3~5 个，用于待归纳卡片快速点选）
    var candidateAuthorChips: [String] = []

    var id: String { entry.path }
    var isAssigned: Bool {
        guard let targetVaultPath else { return false }
        return !targetVaultPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var finalMoveTargetPath: String? {
        guard let dir = targetVaultPath, !dir.isEmpty else { return nil }
        let trimmedDir = dir.hasSuffix("/") ? String(dir.dropLast()) : dir
        return "\(trimmedDir)/\(entry.name)"
    }
}

// MARK: - 多级分类树节点

struct VaultCategoryNode: Identifiable, Hashable {
    let id: String
    let rawName: String
    let cleanName: String
    let fullDisplayName: String
    let path: String
    let parentId: String?
    var children: [VaultCategoryNode] = []
}

// MARK: - 已整理库作者索引实体

struct VaultAuthorEntry: Identifiable, Hashable {
    let id: String
    let canonicalName: String
    let vaultPath: String?
    let categoryPath: String?
    let aliases: [String]
}

// MARK: - 智能收纳分析引擎（对齐 Web mediaOrganizer.ts）

enum MediaOrganizer {
    /// 通用降噪停用词（全小写）
    static let universalStopwords: Set<String> = [
        // 1. 画质与编码规格
        "1080p", "720p", "4k", "2k", "2160p", "60fps", "120fps", "hdr", "hevc", "avc",
        "h264", "h265", "x264", "x265", "10bit", "8bit", "mp4", "mkv", "avi", "wmv",
        "flv", "mov", "zip", "rar", "7z",
        // 2. 字幕、汉化与制作标签
        "中文字幕", "中字", "熟肉", "生肉", "个人汉化", "汉化", "精翻", "机翻", "双语", "简繁",
        "无修", "去码", "步兵", "骑兵", "ai修复", "ai超分", "完全版", "合集", "重置版", "重制版",
        "特典", "未删减", "短视频", "汉化组", "字幕组", "制作组", "扫图组", "搬运", "首发",
        "原档", "无水印", "流出", "泄密", "最新", "顶尖", "极品", "女神", "福利", "资源",
        // 3. 平台与常见泛标签
        "dl版", "fanbox", "patreon", "fantia", "booth", "gumroad", "ci-en",
        "c100", "c101", "c102", "c103", "c104", "c99", "c98", "c97", "c96", "comic1", "comiket",
        "微博", "推特", "丝袜", "网袜", "情趣丝袜", "黑丝", "白丝", "裸舞", "巨乳", "大奶",
        "肥臀", "美臀", "蜜桃臀", "御姐", "萝莉", "少女", "可爱", "女同", "女女", "互玩",
        "可甜可盐", "童颜巨乳", "九头身", "天然豪乳", "大尺度", "网黄", "网红", "福利姬",
        "顶级", "反差", "母狗", "校花", "学生妹", "嫩妹", "口爆", "内射", "中出", "自拍",
        // 4. 系统与占位关键字
        "pending", "未分类", "新建文件夹", "新建文件", "download", "inbox", "vault",
    ]

    /// 常见分类同义词与平台标签映射字典
    static let categorySynonyms: [String: [String]] = [
        "推特": ["twitter", "x.com", "推特", "推特网红", "推"],
        "twitter": ["twitter", "x.com", "推特", "推特网红"],
        "国产": ["国产", "国模", "自拍", "原创", "国产精品", "国产原创", "内陆", "大陆", "华语", "传媒"],
        "探花": ["探花", "探花系列", "酒店", "约拍", "同城", "偷拍", "街拍", "大保健"],
        "偷拍": ["偷拍", "厕拍", "抄底", "街拍"],
        "自拍": ["自拍", "泄密", "流出", "情侣", "夫妻"],
        "日韩": ["日韩", "日本", "韩国", "韩剧", "韩美", "jk", "女优", "av", "jav"],
        "日本": ["日本", "日韩", "女优", "av", "jav"],
        "欧美": ["欧美", "西方", "外模", "洋妞", "white", "ebony"],
        "cosplay": ["cosplay", "cos", "角色扮演", "动漫cos", "二次元cos", "写真", "套图"],
        "动漫": ["动漫", "同人", "二次元", "动画", "漫画", "acg", "番剧", "cg", "anime"],
        "麻豆": ["麻豆", "麻豆传媒", "md传媒", "madou"],
        "糖心": ["糖心", "糖心vlog", "txvlog", "糖心原档"],
        "onlyfans": ["onlyfans", "of", "fansly"],
        "91": ["91", "91制片厂", "91porn", "91大神"],
        "fanbox": ["fanbox", "pixiv", "fantia", "patreon", "ci-en", "booth"],
    ]

    static func cleanCategoryName(_ raw: String) -> String {
        let leaf = raw.split(separator: "/").last.map(String.init) ?? raw
        let noBrace = leaf.trimmingCharacters(in: CharacterSet(charactersIn: "{}｛｝ "))
        // 去除前缀编号，如 "1-国产"、"1 -女优合集"、"99-平台合集"
        if let range = noBrace.range(of: "^\\d+\\s*[-_.]\\s*", options: .regularExpression) {
            let stripped = String(noBrace[range.upperBound...]).trimmingCharacters(in: .whitespaces)
            if !stripped.isEmpty { return stripped }
        }
        return noBrace
    }

    static func isStopword(_ text: String, customWords: Set<String>) -> Bool {
        let t = text.trimmingCharacters(in: CharacterSet(charactersIn: "#@ \t\r\n")).lowercased()
        if t.isEmpty || t.count < 2 { return true }
        if universalStopwords.contains(t) || customWords.contains(t) { return true }
        // 纯数字 / 日期 / 编号
        if t.range(of: "^[0-9._\\-年月日号]+$", options: .regularExpression) != nil { return true }
        // 分辨率规格（如 1920x1080）
        if t.range(of: "^\\d{3,4}[x*]\\d{3,4}$", options: .regularExpression) != nil { return true }
        // 媒体数量标记（如 33v, 319p, 8v+12p, 65套）
        if t.range(of: "^\\d+[vp套集部张]+$", options: .regularExpression) != nil { return true }
        // 字幕/汉化组后缀
        if t.hasSuffix("汉化组") || t.hasSuffix("字幕组") || t.hasSuffix("制作组") || t.hasSuffix("扫图组") {
            return true
        }
        return false
    }

    static func splitAuthorAliases(_ authorName: String, customWords: Set<String>) -> [String] {
        let clean = authorName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return [] }
        var aliases: [String] = []
        var seen: Set<String> = []

        func push(_ candidate: String) {
            let sub = candidate.trimmingCharacters(in: CharacterSet(charactersIn: "#【】[]()（） "))
            let key = sub.lowercased()
            if sub.count >= 2 && !isStopword(sub, customWords: customWords) && !seen.contains(key) {
                seen.insert(key)
                aliases.append(sub)
            }
        }

        // 1. 提取圆括号内别名，如 "Tony (T2)"
        if let regex = try? NSRegularExpression(pattern: "[（(]([^()（）]+)[)）]") {
            let ns = clean as NSString
            for match in regex.matches(in: clean, range: NSRange(location: 0, length: ns.length)) where match.numberOfRanges > 1 {
                push(ns.substring(with: match.range(at: 1)))
            }
        }
        let withoutParen = clean.replacingOccurrences(of: "[（(][^()（）]+[)）]", with: " ", options: .regularExpression)

        // 2. 按常见分隔符切片：/、／、\、&、+、×、*、|、_、·、顿号
        let parts = withoutParen.components(separatedBy: CharacterSet(charactersIn: "/／\\&+×*|_·、"))
        for p in parts {
            push(p)
        }
        return aliases
    }

    /// 提取所有被括号包裹的标签片段（【】[]「」『』()（）），自动去除前导 # 并过滤停用词
    static func extractBracketTokens(_ name: String, customWords: Set<String>) -> [String] {
        guard !name.isEmpty else { return [] }
        let pattern = "[【\\[「『(（]([^【】\\[\\]「」『』()（）]+)[】\\]」』)）]"
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return [] }
        let ns = name as NSString
        var result: [String] = []
        var seen: Set<String> = []
        for match in regex.matches(in: name, range: NSRange(location: 0, length: ns.length)) where match.numberOfRanges > 1 {
            let raw = ns.substring(with: match.range(at: 1))
                .trimmingCharacters(in: CharacterSet(charactersIn: "# \t"))
            let key = raw.lowercased()
            if !raw.isEmpty && !isStopword(raw, customWords: customWords) && !seen.contains(key) {
                seen.insert(key)
                result.append(raw)
            }
        }
        return result
    }

    /// 从文件名提取 3~5 个最像作者名的候选胶囊（括号词优先，其次 #标签 和 ｜分隔片段）
    static func extractAuthorCandidates(_ name: String, customWords: Set<String>) -> [String] {
        let baseName: String
        if let dot = name.lastIndex(of: "."), name.distance(from: dot, to: name.endIndex) <= 5 {
            baseName = String(name[..<dot])
        } else {
            baseName = name
        }

        var candidates: [String] = []
        var seen: Set<String> = []

        func add(_ raw: String) {
            let cleaned = raw.trimmingCharacters(in: CharacterSet(charactersIn: "#@【】[]「」『』()（）｜|_- "))
            let key = cleaned.lowercased()
            guard cleaned.count >= 2, cleaned.count <= 22,
                  !isStopword(cleaned, customWords: customWords),
                  !seen.contains(key) else { return }
            seen.insert(key)
            candidates.append(cleaned)
        }

        // 1. 括号内词条最高优先级（如 【#小仙云】、「一只顶美」）
        for token in extractBracketTokens(baseName, customWords: customWords) {
            add(token)
        }

        // 2. 竖线分隔片段（如 頂級嫵媚女神｜Beauty0827｜...）
        if baseName.contains("｜") || baseName.contains("|") {
            let segs = baseName.components(separatedBy: CharacterSet(charactersIn: "｜|"))
            for seg in segs {
                add(seg)
            }
        }

        // 3. #Hashtag 标签（过滤掉泛标签后剩下的多半是博主名，如 #魔鬼爱 #Uaijie）
        if let hashRegex = try? NSRegularExpression(pattern: "#([^\\s#【】\\[\\]「」()（）]+)") {
            let ns = baseName as NSString
            for match in hashRegex.matches(in: baseName, range: NSRange(location: 0, length: ns.length)) where match.numberOfRanges > 1 {
                add(ns.substring(with: match.range(at: 1)))
            }
        }

        return Array(candidates.prefix(5))
    }

    // MARK: - 分类树构建与最深层匹配

    static func buildCategoryTree(from categories: [DomainMetaCategory]) -> [VaultCategoryNode] {
        var nodeMap: [String: VaultCategoryNode] = [:]
        for cat in categories {
            guard let remotePath = cat.remotePath, !remotePath.isEmpty else { continue }
            let clean = cleanCategoryName(cat.name)
            let fullDisplay = cat.name
                .split(separator: "/")
                .map { cleanCategoryName(String($0)) }
                .joined(separator: " / ")
            nodeMap[cat.id] = VaultCategoryNode(
                id: cat.id,
                rawName: cat.name,
                cleanName: clean,
                fullDisplayName: fullDisplay,
                path: remotePath,
                parentId: cat.parentId,
                children: []
            )
        }

        // 递归组装父子层级
        func attachChildren(for node: VaultCategoryNode) -> VaultCategoryNode {
            var copy = node
            let directChildren = nodeMap.values
                .filter { $0.parentId == node.id }
                .sorted { $0.rawName.localizedStandardCompare($1.rawName) == .orderedAscending }
                .map { attachChildren(for: $0) }
            copy.children = directChildren
            return copy
        }

        let roots = nodeMap.values
            .filter { node in
                guard let pid = node.parentId, !pid.isEmpty else { return true }
                return nodeMap[pid] == nil
            }
            .sorted { $0.rawName.localizedStandardCompare($1.rawName) == .orderedAscending }
            .map { attachChildren(for: $0) }

        return roots
    }

    static func flattenCategories(_ nodes: [VaultCategoryNode]) -> [VaultCategoryNode] {
        var out: [VaultCategoryNode] = []
        for node in nodes {
            out.append(node)
            out.append(contentsOf: flattenCategories(node.children))
        }
        return out
    }

    private static func isCategoryNameMatched(_ cleanNodeName: String, in lowerText: String) -> Bool {
        let lowerNode = cleanNodeName.lowercased()
        if lowerNode.count >= 2 && lowerText.contains(lowerNode) {
            return true
        }
        for (key, synonyms) in categorySynonyms {
            if lowerNode.contains(key) || key.contains(lowerNode) {
                if synonyms.contains(where: { lowerText.contains($0) }) {
                    return true
                }
            }
        }
        return false
    }

    /// 优先命中树中最深层的子分类
    static func findDeepestMatchedCategory(
        for text: String,
        tree: [VaultCategoryNode],
        customWords: Set<String>
    ) -> VaultCategoryNode? {
        let lower = text.lowercased()
        var best: (node: VaultCategoryNode, depth: Int)? = nil

        func dfs(_ nodes: [VaultCategoryNode], depth: Int) {
            for node in nodes {
                if !isStopword(node.cleanName, customWords: customWords),
                   isCategoryNameMatched(node.cleanName, in: lower) {
                    if best == nil || depth > best!.depth || (depth == best!.depth && node.cleanName.count > best!.node.cleanName.count) {
                        best = (node, depth)
                    }
                }
                dfs(node.children, depth: depth + 1)
            }
        }

        dfs(tree, depth: 1)
        return best?.node
    }

    // MARK: - 作者索引构建（优先已整理库真实目录路径）

    static func buildAuthorEntries(
        from authors: [DomainMetaAuthor],
        customWords: Set<String>
    ) -> [VaultAuthorEntry] {
        authors.map { a in
            let explicit = (a.aliases ?? []).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
            let split = splitAuthorAliases(a.name, customWords: customWords)
            var merged: [String] = []
            var seen: Set<String> = [a.name.lowercased()]
            for item in explicit + split {
                let key = item.lowercased()
                if !seen.contains(key) {
                    seen.insert(key)
                    merged.append(item)
                }
            }
            let catPath: String?
            if let vp = a.vaultPath, let slash = vp.lastIndex(of: "/") {
                catPath = String(vp[..<slash])
            } else {
                catPath = nil
            }
            return VaultAuthorEntry(
                id: a.id,
                canonicalName: a.name,
                vaultPath: a.vaultPath,
                categoryPath: catPath,
                aliases: merged
            )
        }
    }

    // MARK: - 核心单项分析器：过滤词 -> 优先匹配作者(进已归纳) -> 未命中进待归纳(自动记分类 + 推荐作者词)

    static func analyzePendingEntry(
        _ entry: RemoteBrowserEntry,
        authors: [VaultAuthorEntry],
        categoryTree: [VaultCategoryNode],
        vaultRoot: String,
        customFilterWords: [String],
        savedAssignments: [String: CuratorItemAssignment?]
    ) -> PendingCurationItem {
        let customSet = Set(customFilterWords.map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty })
        let candidates = extractAuthorCandidates(entry.name, customWords: customSet)

        // 0. 优先恢复服务端保存的已归纳草稿
        if let optionalSaved = savedAssignments[entry.path],
           let saved = optionalSaved,
           let savedTarget = saved.targetVaultPath,
           !savedTarget.isEmpty {
            return PendingCurationItem(
                entry: entry,
                assignmentSource: .restoredDraft,
                matchedAuthorId: nil,
                matchedAuthor: saved.author,
                matchedAlias: nil,
                matchedCategoryPath: saved.categoryPath,
                matchedCategoryName: saved.categoryName,
                targetVaultPath: savedTarget,
                candidateAuthorChips: candidates
            )
        }

        // 先对文件名移除自定义过滤词再匹配
        var cleanedName = entry.name
        for fw in customFilterWords where !fw.trimmingCharacters(in: .whitespaces).isEmpty {
            cleanedName = cleanedName.replacingOccurrences(of: fw, with: " ", options: .caseInsensitive)
        }

        // 仅在已整理库存在真实目录路径的作者中优先匹配（避免漫画作者干扰）
        let vaultAuthors = authors.filter { $0.vaultPath != nil && !($0.vaultPath?.isEmpty ?? true) }

        // 构建快速索引
        var canonicalMap: [String: VaultAuthorEntry] = [:]
        var aliasMap: [String: (author: VaultAuthorEntry, alias: String)] = [:]
        for author in vaultAuthors {
            let key = author.canonicalName.trimmingCharacters(in: CharacterSet(charactersIn: "【】[]()（）# ")).lowercased()
            if !key.isEmpty { canonicalMap[key] = author }
            for alias in author.aliases {
                let aKey = alias.lowercased()
                if aKey.count >= 2 && aliasMap[aKey] == nil {
                    aliasMap[aKey] = (author, alias)
                }
            }
        }

        // 1. 优先通过括号词条精准匹配已有作者 / 别名
        let bracketTokens = extractBracketTokens(cleanedName, customWords: customSet)
        for token in bracketTokens {
            let key = token.lowercased()
            if let hit = canonicalMap[key], let vp = hit.vaultPath {
                return PendingCurationItem(
                    entry: entry,
                    assignmentSource: .matchedAuthor,
                    matchedAuthorId: hit.id,
                    matchedAuthor: hit.canonicalName,
                    matchedAlias: nil,
                    matchedCategoryPath: hit.categoryPath,
                    matchedCategoryName: hit.categoryPath.map { cleanCategoryName($0) },
                    targetVaultPath: vp,
                    candidateAuthorChips: candidates
                )
            }
            if let aliasHit = aliasMap[key], let vp = aliasHit.author.vaultPath {
                return PendingCurationItem(
                    entry: entry,
                    assignmentSource: .matchedAlias,
                    matchedAuthorId: aliasHit.author.id,
                    matchedAuthor: aliasHit.author.canonicalName,
                    matchedAlias: aliasHit.alias,
                    matchedCategoryPath: aliasHit.author.categoryPath,
                    matchedCategoryName: aliasHit.author.categoryPath.map { cleanCategoryName($0) },
                    targetVaultPath: vp,
                    candidateAuthorChips: candidates
                )
            }
        }

        // 2. 全文件名包含匹配（按作者名长度降序，避免短名误伤）
        let lowerCleaned = cleanedName.lowercased()
        for author in vaultAuthors.sorted(by: { $0.canonicalName.count > $1.canonicalName.count }) where author.canonicalName.count >= 2 {
            let cleanAuthor = author.canonicalName.trimmingCharacters(in: CharacterSet(charactersIn: "【】[]()（）# ")).lowercased()
            if !isStopword(cleanAuthor, customWords: customSet),
               lowerCleaned.contains(cleanAuthor),
               let vp = author.vaultPath {
                return PendingCurationItem(
                    entry: entry,
                    assignmentSource: .matchedAuthor,
                    matchedAuthorId: author.id,
                    matchedAuthor: author.canonicalName,
                    matchedAlias: nil,
                    matchedCategoryPath: author.categoryPath,
                    matchedCategoryName: author.categoryPath.map { cleanCategoryName($0) },
                    targetVaultPath: vp,
                    candidateAuthorChips: candidates
                )
            }
        }
        // 别名包含匹配
        let sortedAliases = aliasMap.values.sorted { $0.alias.count > $1.alias.count }
        for item in sortedAliases where item.alias.count >= 2 {
            let lowerAlias = item.alias.lowercased()
            if !isStopword(lowerAlias, customWords: customSet),
               lowerCleaned.contains(lowerAlias),
               let vp = item.author.vaultPath {
                return PendingCurationItem(
                    entry: entry,
                    assignmentSource: .matchedAlias,
                    matchedAuthorId: item.author.id,
                    matchedAuthor: item.author.canonicalName,
                    matchedAlias: item.alias,
                    matchedCategoryPath: item.author.categoryPath,
                    matchedCategoryName: item.author.categoryPath.map { cleanCategoryName($0) },
                    targetVaultPath: vp,
                    candidateAuthorChips: candidates
                )
            }
        }

        // 3. 未命中作者 -> 进入「待归纳」；同时尝试自动识别最深层分类并预记
        let matchedCat = findDeepestMatchedCategory(for: cleanedName, tree: categoryTree, customWords: customSet)
        return PendingCurationItem(
            entry: entry,
            assignmentSource: .none,
            matchedAuthorId: nil,
            matchedAuthor: nil,
            matchedAlias: nil,
            matchedCategoryPath: matchedCat?.path,
            matchedCategoryName: matchedCat?.fullDisplayName,
            targetVaultPath: nil,
            candidateAuthorChips: candidates
        )
    }
}
