import Foundation
import SwiftUI

/// 分类/文件夹显示名：只保留最后一级，并去掉 “1-”/“1_”/“1.” 这类排序前缀。
func pmlDisplayName(_ raw: String) -> String {
    let last = raw.split(separator: "/").last.map(String.init) ?? raw
    let cleaned = last.replacingOccurrences(
        of: #"^\s*\d+\s*[-_.、]\s*"#,
        with: "",
        options: .regularExpression
    )
    return cleaned.isEmpty ? last : cleaned
}

struct AuthorFolderItem: Identifiable {
    let id: String
    let name: String
    let count: Int
}

struct AuthorFolderSnapshot {
    let root: LibraryRoot
    let basePath: String
    let folders: [AuthorFolderItem]
    let files: [LibraryAsset]
}

/// 选择作者作品最多的根目录，并取这些作品的公共父目录，直接展示该层内容。
func pmlAuthorSnapshot(assets: [LibraryAsset], roots: [LibraryRoot]) -> AuthorFolderSnapshot? {
    let grouped = Dictionary(grouping: assets, by: { $0.rootId })
    guard let primary = grouped.max(by: { $0.value.count < $1.value.count }),
          let root = roots.first(where: { $0.id == primary.key }) else { return nil }

    let basePrefix = root.remotePath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    func components(_ asset: LibraryAsset) -> [String]? {
        let full = asset.remotePath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard full.hasPrefix(basePrefix) else { return nil }
        let rel = String(full.dropFirst(basePrefix.count))
            .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return rel.split(separator: "/").map(String.init)
    }

    var common: [String]? = nil
    for asset in primary.value {
        guard let parts = components(asset), parts.count > 1 else { continue }
        let dirs = Array(parts.dropLast())
        if common == nil {
            common = dirs
        } else if var current = common {
            var prefix: [String] = []
            for (lhs, rhs) in zip(current, dirs) where lhs == rhs {
                prefix.append(lhs)
            }
            current = prefix
            common = current
        }
    }
    let base = common ?? []
    let basePath = base.joined(separator: "/")

    var folderCounts: [String: Int] = [:]
    var files: [LibraryAsset] = []
    for asset in primary.value {
        guard let parts = components(asset) else { continue }
        let remainder = Array(parts.dropFirst(base.count))
        if remainder.count >= 2 {
            folderCounts[remainder[0], default: 0] += 1
        } else if remainder.count == 1 {
            files.append(asset)
        }
    }
    let folders = folderCounts.map {
        AuthorFolderItem(id: "\(root.id)|\(basePath)|\($0.key)", name: $0.key, count: $0.value)
    }
    .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }

    return AuthorFolderSnapshot(root: root, basePath: basePath, folders: folders, files: files)
}
/// 单个分类页：子分类递归进入，作者列表在下方；系统返回键自动回到上一级。
struct CreatorCategoryView: View {
    let category: LibraryCategory?

    @EnvironmentObject private var model: AppModel
    @State private var children: [LibraryCategory] = []
    @State private var authors: [LibraryAuthor] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var searchText = ""
    @State private var sortByMedia = true

    private var displayedAuthors: [LibraryAuthor] {
        let filtered = searchText.isEmpty
            ? authors
            : authors.filter {
                $0.name.localizedCaseInsensitiveContains(searchText)
                    || $0.aliases.contains(where: { $0.localizedCaseInsensitiveContains(searchText) })
            }
        return filtered.sorted {
            sortByMedia ? $0.mediaCount > $1.mediaCount
                        : $0.name.localizedStandardCompare($1.name) == .orderedAscending
        }
    }

    var body: some View {
        List {
            if loading && children.isEmpty && authors.isEmpty {
                HStack { ProgressView(); Text("加载中…").foregroundStyle(.secondary) }
            }

            if !children.isEmpty {
                Section("子分类") {
                    ForEach(children) { child in
                        NavigationLink {
                            CreatorCategoryView(category: child)
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "folder.fill").foregroundStyle(.tint)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(pmlDisplayName(child.name))
                                    Text("\(child.authorCount) 位创作者 · \(child.mediaCount) 项")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                        }
                    }
                }
            }

            if !displayedAuthors.isEmpty {
                Section("创作者") {
                    ForEach(displayedAuthors) { author in
                        NavigationLink {
                            AuthorProfileView(author: author)
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "person.crop.circle.fill")
                                    .font(.title3)
                                    .foregroundStyle(.secondary)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(author.name)
                                    Text("\(author.mediaCount) 项作品")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                        }
                    }
                }
            }

            if !loading && children.isEmpty && displayedAuthors.isEmpty && errorText.isEmpty {
                Text("无内容").foregroundStyle(.secondary)
            }
            if !errorText.isEmpty {
                Text(errorText).font(.footnote).foregroundStyle(.red)
            }
        }
        .navigationTitle(pmlDisplayName(category?.name ?? "全部创作者"))
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "搜索创作者")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Picker("排序", selection: $sortByMedia) {
                        Text("作品数").tag(true)
                        Text("名称").tag(false)
                    }
                } label: {
                    Image(systemName: "arrow.up.arrow.down")
                }
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            if let category {
                children = try await client.libraryCategories(parentId: category.id, filter: "all")
            }
            authors = try await client.libraryAuthors(categoryId: category?.id, filter: "all")
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loading = false
    }
}

/// 作者作品的真实目录层级：根目录 → 文件夹 → … → 文件。
struct AuthorFolderView: View {
    let author: LibraryAuthor
    let allAssets: [LibraryAsset]
    let root: LibraryRoot
    let relativePath: String

    @EnvironmentObject private var model: AppModel
    @State private var preview: PreviewItem?
    @State private var errorText = ""

    private struct Folder: Identifiable {
        let id: String
        let name: String
        let count: Int
    }

    private var currentPrefix: String {
        relativePath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }

    private var folders: [Folder] {
        var counts: [String: Int] = [:]
        for asset in matchingAssets {
            guard let remainder = remainderPath(asset), !remainder.isEmpty else { continue }
            if let slash = remainder.firstIndex(of: "/") {
                let name = String(remainder[..<slash])
                counts[name, default: 0] += 1
            }
        }
        return counts.map { key, value in
            Folder(id: "\(root.id)|\(currentPrefix)|\(key)", name: key, count: value)
        }
        .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    private var files: [LibraryAsset] {
        matchingAssets.filter { remainderPath($0)?.contains("/") == false }
            .sorted { $0.fileName.localizedStandardCompare($1.fileName) == .orderedAscending }
    }

    private var matchingAssets: [LibraryAsset] {
        allAssets.filter { $0.rootId == root.id }
    }

    var body: some View {
        List {
            if folders.isEmpty && files.isEmpty {
                Text("无文件").foregroundStyle(.secondary)
            }

            if !folders.isEmpty {
                Section("文件夹") {
                    ForEach(folders) { folder in
                        NavigationLink {
                            AuthorFolderView(
                                author: author,
                                allAssets: allAssets,
                                root: root,
                                relativePath: join(currentPrefix, folder.name)
                            )
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "folder.fill").foregroundStyle(.tint)
                                Text(pmlDisplayName(folder.name))
                                Spacer()
                                Text("\(folder.count)").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }

            if !files.isEmpty {
                Section("文件") {
                    ForEach(files) { asset in
                        Button {
                            Task { await open(asset) }
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: asset.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                                    .foregroundStyle(.secondary)
                                Text(asset.fileName).lineLimit(1)
                            }
                        }
                    }
                }
            }

            if !errorText.isEmpty {
                Text(errorText).font(.footnote).foregroundStyle(.red)
            }
        }
        .navigationTitle(currentPrefix.isEmpty ? root.kindLabel : pmlDisplayName(String(currentPrefix.split(separator: "/").last ?? "")))
        .navigationBarTitleDisplayMode(.inline)
        .fullScreenCover(item: $preview) { item in
            if item.isVideo {
                GestureVideoPlayerSheet(
                    url: item.url,
                    title: item.title,
                    assetId: item.assetId,
                    startAt: item.startAt,
                    fallbackURL: item.fallbackURL,
                    sourceLabel: item.sourceLabel
                )
                .environmentObject(model)
            } else {
                ImageViewerSheet(url: item.url)
            }
        }
    }

    private func remainderPath(_ asset: LibraryAsset) -> String? {
        let base = root.remotePath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let full = asset.remotePath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        guard full.hasPrefix(base) else { return nil }
        var rel = String(full.dropFirst(base.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if !currentPrefix.isEmpty {
            guard rel == currentPrefix || rel.hasPrefix(currentPrefix + "/") else { return nil }
            rel = String(rel.dropFirst(currentPrefix.count)).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        }
        return rel
    }

    private func join(_ lhs: String, _ rhs: String) -> String {
        lhs.isEmpty ? rhs : "\(lhs)/\(rhs)"
    }

    private func open(_ asset: LibraryAsset) async {
        guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else {
            errorText = "无法获取播放地址"
            return
        }
        preview = PreviewItem(
            url: source.url,
            isVideo: asset.mediaType == "video",
            assetId: asset.id,
            title: asset.fileName,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: asset.remotePath
        )
    }
}
