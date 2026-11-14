import PhotosUI
import SwiftUI
import UIKit

private enum AuthorSortMode: String, CaseIterable, Identifiable {
    case mediaCount = "作品数"
    case name = "名称"
    var id: String { rawValue }
}

/// 创作者分类大厅：一级分类卡片 → 子分类/作者列表，可切换全库索引。
struct AuthorsView: View {
    @EnvironmentObject private var model: AppModel

    @State private var categories: [LibraryCategory] = []
    @State private var authors: [LibraryAuthor] = []
    @State private var selectedCategory: LibraryCategory?
    @State private var viewAll = false
    @State private var loading = false
    @State private var checking = false
    @State private var refreshNotice = ""
    @State private var errorText = ""
    @State private var searchText = ""
    @State private var sortMode: AuthorSortMode = .mediaCount
    @AppStorage("pml.authors.cardLayout") private var cardLayout = "grid"
    @StateObject private var thumbs = ThumbnailLoader()

    private var isHall: Bool {
        selectedCategory == nil && !viewAll && searchText.isEmpty
    }

    private var topCategories: [LibraryCategory] {
        let ids = Set(categories.map(\.id))
        return categories
            .filter { category in
                guard let parent = category.parentId, !parent.isEmpty else { return true }
                return !ids.contains(parent)
            }
            .sorted { $0.authorCount > $1.authorCount }
    }

    private var childCategories: [LibraryCategory] {
        guard let selectedCategory else { return [] }
        return categories
            .filter { $0.parentId == selectedCategory.id }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    private var displayedAuthors: [LibraryAuthor] {
        let filtered: [LibraryAuthor]
        if searchText.isEmpty {
            filtered = authors
        } else {
            filtered = authors.filter { author in
                author.name.localizedCaseInsensitiveContains(searchText)
                    || author.aliases.contains(where: { $0.localizedCaseInsensitiveContains(searchText) })
            }
        }
        switch sortMode {
        case .mediaCount:
            return filtered.sorted { $0.mediaCount > $1.mediaCount }
        case .name:
            return filtered.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    if loading && categories.isEmpty && authors.isEmpty {
                        HStack(spacing: 10) {
                            ProgressView()
                            Text("加载创作者…").foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.top, 40)
                    } else if isHall {
                        hall
                    } else {
                        detail
                    }

                    if !refreshNotice.isEmpty {
                        Text(refreshNotice)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }

                    if !errorText.isEmpty {
                        Text(errorText)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
            }
            .navigationTitle(navigationTitle)
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "搜索创作者/别名")
            .refreshable { await refreshAndCheckProvider() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 12) {
                        Button {
                            Task { await refreshAndCheckProvider() }
                        } label: {
                            if checking {
                                ProgressView()
                            } else {
                                Image(systemName: "arrow.clockwise")
                            }
                        }
                        .disabled(checking)
                        .accessibilityLabel("检查 CD2 更新")

                        if isHall {
                            NavigationLink("全库索引") {
                                CreatorCategoryView(category: nil)
                            }
                        } else {
                            Button {
                                cardLayout = cardLayout == "grid" ? "list" : "grid"
                            } label: {
                                Image(systemName: cardLayout == "grid" ? "list.bullet" : "square.grid.2x2")
                            }
                            .accessibilityLabel(cardLayout == "grid" ? "切换为单列" : "切换为双列")
                            Menu {
                                Picker("排序", selection: $sortMode) {
                                    ForEach(AuthorSortMode.allCases) { mode in
                                        Text(mode.rawValue).tag(mode)
                                    }
                                }
                            } label: {
                                Image(systemName: "arrow.up.arrow.down")
                            }
                            Button("分类总览") {
                                selectedCategory = nil
                                viewAll = false
                                searchText = ""
                                authors = []
                                Task { await load() }
                            }
                        }
                    }
                }
            }
            .task { await load() }
            .onChange(of: searchText) { _, value in
                if !value.isEmpty && !viewAll && selectedCategory == nil {
                    viewAll = true
                    Task { await load() }
                }
            }
        }
    }

    private var navigationTitle: String {
        if isHall { return "创作者" }
        if viewAll { return "全部创作者" }
        return selectedCategory?.name ?? "创作者"
    }

    private var hall: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                Text("创作者分类大厅")
                    .font(.title3.weight(.semibold))
                Spacer()
                Text("\(topCategories.count) 个分类 · \(topCategories.reduce(0) { $0 + Int($1.authorCount) }) 位创作者")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if topCategories.isEmpty {
                Text("还没有分类。扫描资料库并整理后，这里会显示分类大厅。")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .padding(.top, 30)
            } else {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)], spacing: 10) {
                    ForEach(topCategories) { category in
                        categoryCard(category)
                    }
                }
            }
        }
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                Text(viewAll ? "全部创作者" : (selectedCategory?.name ?? "创作者"))
                    .font(.title3.weight(.semibold))
                Spacer()
                Text("\(displayedAuthors.count) 位")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if !childCategories.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(childCategories) { child in
                            Button {
                                selectedCategory = child
                                Task {
                                    await loadChildren(of: child)
                                    await load()
                                }
                            } label: {
                                Text(child.name)
                                    .font(.caption)
                                    .padding(.horizontal, 12)
                                    .padding(.vertical, 7)
                                    .background(Color(.secondarySystemBackground), in: Capsule())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }

            if loading && authors.isEmpty {
                HStack(spacing: 10) {
                    ProgressView()
                    Text("加载作者…").foregroundStyle(.secondary)
                }
                .padding(.top, 20)
            } else if displayedAuthors.isEmpty {
                Text("这个分类下还没有创作者。")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .padding(.top, 30)
            } else if cardLayout == "list" {
                LazyVStack(spacing: 14) {
                    ForEach(displayedAuthors) { author in
                        authorCardLink(author, bannerHeight: 190)
                    }
                }
            } else {
                LazyVGrid(
                    columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)],
                    spacing: 14
                ) {
                    ForEach(displayedAuthors) { author in
                        authorCardLink(author, bannerHeight: 108)
                    }
                }
            }
        }
    }

    private func authorCardLink(_ author: LibraryAuthor, bannerHeight: CGFloat) -> some View {
        NavigationLink {
            AuthorProfileView(author: author)
        } label: {
            AuthorShowcaseCard(
                author: author,
                bannerHeight: bannerHeight,
                isTopCreator: topCreatorIds.contains(author.id)
            )
        }
        .buttonStyle(.plain)
    }

    private var topCreatorIds: Set<String> {
        let pool = authors.filter { $0.mediaCount > 0 }.sorted { $0.mediaCount > $1.mediaCount }
        let cutoff = max(1, Int(ceil(Double(pool.count) * 0.1)))
        return Set(pool.prefix(cutoff).map(\.id))
    }

    private func categoryCard(_ category: LibraryCategory) -> some View {
        NavigationLink {
            CreatorCategoryView(category: category)
        } label: {
            VStack(alignment: .leading, spacing: 8) {
                Image(systemName: "folder.fill")
                    .font(.title3)
                    .foregroundStyle(.tint)
                Text(pmlDisplayName(category.name))
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                Text("\(category.authorCount) 位 · \(category.mediaCount) 项")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, minHeight: 110, alignment: .topLeading)
            .padding(12)
            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(.plain)
    }

    private func authorRow(_ author: LibraryAuthor) -> some View {
        HStack(spacing: 12) {
            avatar(author)
                .frame(width: 46, height: 46)
                .clipShape(Circle())

            VStack(alignment: .leading, spacing: 3) {
                Text(author.name).lineLimit(1)
                HStack(spacing: 8) {
                    Text("\(author.mediaCount) 项")
                    if author.collectionCount > 0 {
                        Text("\(author.collectionCount) 个合集")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            Image(systemName: "chevron.right")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 9)
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private func avatar(_ author: LibraryAuthor) -> some View {
        if let id = author.avatarAssetId, let image = thumbs.images[id] {
            Image(uiImage: image).resizable().scaledToFill()
        } else if let id = author.coverAssetId, let image = thumbs.images[id] {
            Image(uiImage: image).resizable().scaledToFill()
        } else {
            ZStack {
                LinearGradient(colors: [.accentColor.opacity(0.85), .accentColor.opacity(0.45)], startPoint: .topLeading, endPoint: .bottomTrailing)
                Text(String(author.name.prefix(1)))
                    .font(.title2.weight(.bold))
                    .foregroundStyle(.white)
            }
        }
    }

    private func loadChildren(of category: LibraryCategory) async {
        guard let client = model.apiClient else { return }
        guard !categories.contains(where: { $0.parentId == category.id }) else { return }
        if let children = try? await client.libraryCategories(parentId: category.id, filter: "all") {
            categories.append(contentsOf: children)
        }
    }

    /// Pull-to-refresh and the toolbar button use the same real provider check.
    private func refreshAndCheckProvider() async {
        guard let client = model.apiClient, !checking else { return }
        checking = true
        errorText = ""
        do {
            let outcomes = try await client.checkDomainRootsForUpdates()
            await load()
            refreshNotice = outcomes.refreshNotice
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
        checking = false
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            if categories.isEmpty {
                categories = try await client.libraryCategories(filter: "all")
            }
            if isHall {
                authors = []
            } else {
                authors = try await client.libraryAuthors(categoryId: selectedCategory?.id, filter: "all")
            }
            if categories.isEmpty && authors.isEmpty {
                viewAll = true
                authors = try await client.libraryAuthors(filter: "all")
            }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }
}
struct AuthorProfileView: View {
    let author: LibraryAuthor

    @EnvironmentObject private var model: AppModel
    @State private var assets: [LibraryAsset] = []
    @State private var roots: [LibraryRoot] = []
    @State private var folderRoot: LibraryRoot?
    @State private var folderPath = ""
    @State private var folderEntries: [RemoteBrowserEntry] = []
    @State private var folderLoading = false
    @State private var layout: BrowseLayout = .list
    @State private var sort: BrowseSort = .name
    @ObservedObject private var covers = ThumbnailService.shared
    @State private var loading = false
    @State private var errorText = ""
    @State private var preview: PreviewItem?
    @State private var showAliasEditor = false
    @State private var aliasText = ""
    @State private var notice = ""
    @State private var displayAuthor: LibraryAuthor?
    @State private var avatarItem: PhotosPickerItem?
    @State private var bannerItem: PhotosPickerItem?
    @State private var showAvatarPicker = false
    @State private var showBannerPicker = false
    @StateObject private var thumbs = ThumbnailLoader()

    private let columns = [GridItem(.adaptive(minimum: 100), spacing: 8)]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header

                if !errorText.isEmpty {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                }

                if assets.isEmpty && !loading {
                    Text("暂无作品").foregroundStyle(.secondary)
                }

                if folderLoading {
                    HStack(spacing: 8) {
                        ProgressView()
                        Text("加载目录…").font(.footnote).foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 8)
                } else if let folderRoot {
                    folderContent(root: folderRoot)
                }
            }
            .padding(16)
        }
        .navigationTitle(author.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button {
                        showAvatarPicker = true
                    } label: {
                        Label("更换头像", systemImage: "person.crop.circle")
                    }
                    Button {
                        showBannerPicker = true
                    } label: {
                        Label("更换横幅", systemImage: "rectangle.topthird.inset.filled")
                    }
                    Button {
                        aliasText = info.aliases.joined(separator: "，")
                        showAliasEditor = true
                    } label: {
                        Label("编辑别名", systemImage: "tag")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                }
            }
        }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Picker("视图", selection: $layout) {
                        ForEach(BrowseLayout.allCases) { item in
                            Label(item.title, systemImage: item.symbol).tag(item)
                        }
                    }
                    Divider()
                    Picker("排序", selection: $sort) {
                        ForEach(BrowseSort.allCases) { item in
                            Text(item.title).tag(item)
                        }
                    }
                } label: {
                    Image(systemName: layout.symbol)
                }
            }
        }
        .onChange(of: sort) { _, _ in
            Task { await loadAuthorFolder() }
        }        .photosPicker(isPresented: $showAvatarPicker, selection: $avatarItem, matching: .images)
        .photosPicker(isPresented: $showBannerPicker, selection: $bannerItem, matching: .images)
        .onChange(of: avatarItem) { _, item in
            guard let item else { return }
            Task { await upload(item, isAvatar: true) }
        }
        .onChange(of: bannerItem) { _, item in
            guard let item else { return }
            Task { await upload(item, isAvatar: false) }
        }
        .task { await load() }
        .sheet(isPresented: $showAliasEditor) {
            aliasEditor
        }
        .fullScreenCover(item: $preview) { item in
            if item.isVideo {
                GestureVideoPlayerSheet(
                    url: item.url,
                    title: item.title,
                    assetId: item.assetId,
                    startAt: item.startAt,
                    fallbackURL: item.fallbackURL,
                    sourceLabel: item.sourceLabel,
                    remotePath: item.remotePath
                )
                .id(item.id)
                .environmentObject(model)
            } else {
                ImageViewerSheet(url: item.url)
            }
        }
    }

    private var info: LibraryAuthor {
        displayAuthor ?? author
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let bannerId = info.bannerAssetId, let image = thumbs.images[bannerId] {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .frame(height: 140)
                    .frame(maxWidth: .infinity)
                    .clipped()
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            }

            HStack(spacing: 12) {
                avatarView(info)
                    .frame(width: 54, height: 54)
                    .clipShape(Circle())

                VStack(alignment: .leading, spacing: 3) {
                    Text(info.name).font(.title2.weight(.bold))
                    if !info.aliases.isEmpty {
                        Text(info.aliases.joined(separator: " / "))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Text("\(info.mediaCount) 项作品 · \(info.collectionCount) 个合集")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if !notice.isEmpty {
                Text(notice).font(.caption).foregroundStyle(.secondary)
            }
        }
        .task {
            if let bannerId = info.bannerAssetId {
                await thumbs.image(assetId: bannerId, client: model.apiClient)
            }
            if let avatarId = info.avatarAssetId {
                await thumbs.image(assetId: avatarId, client: model.apiClient)
            }
        }
    }

    private var aliasEditor: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("别名，用逗号分隔", text: $aliasText, axis: .vertical)
                        .lineLimit(2...4)
                } footer: {
                    Text("别名会参与待收纳整理时的作者匹配。")
                }
                Section {
                    Button("保存别名") { Task { await saveAliases() } }
                }
            }
            .navigationTitle("编辑别名")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { showAliasEditor = false }
                }
            }
        }
    }

    @ViewBuilder
    private func avatarView(_ author: LibraryAuthor) -> some View {
        if let id = author.avatarAssetId, let image = thumbs.images[id] {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
        } else {
            ZStack {
                Circle().fill(.quaternary.opacity(0.35))
                Image(systemName: "person.fill").foregroundStyle(.tint)
            }
        }
    }

    private func saveAliases() async {
        guard let client = model.apiClient else { return }
        let aliases = aliasText
            .split(whereSeparator: { "，,;；/".contains($0) })
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        do {
            try await client.updateAuthorAliases(authorId: info.id, aliases: aliases)
            notice = "别名已保存（\(aliases.count) 个）"
            showAliasEditor = false
            await refreshAuthor()
        } catch {
            notice = error.localizedDescription
        }
    }

    private func upload(_ item: PhotosPickerItem, isAvatar: Bool) async {
        guard let client = model.apiClient else { return }
        guard let data = try? await item.loadTransferable(type: Data.self),
              let base64 = pmlJPEGBase64(data, maxDimension: isAvatar ? 800 : 1600) else {
            notice = "读取图片失败"
            return
        }
        do {
            if isAvatar {
                try await client.uploadAuthorAvatar(authorId: info.id, dataBase64: base64)
            } else {
                try await client.uploadAuthorBanner(authorId: info.id, dataBase64: base64)
            }
            notice = isAvatar ? "头像已更新" : "横幅已更新"
            await refreshAuthor()
        } catch {
            notice = error.localizedDescription
        }
    }

    private func refreshAuthor() async {
        guard let client = model.apiClient else { return }
        if let authors = try? await client.libraryAuthors(filter: "all"),
           let latest = authors.first(where: { $0.id == info.id }) {
            displayAuthor = latest
            if let avatarId = latest.avatarAssetId {
                await thumbs.image(assetId: avatarId, client: client)
            }
            if let bannerId = latest.bannerAssetId {
                await thumbs.image(assetId: bannerId, client: client)
            }
        }
    }

    private func cell(_ asset: LibraryAsset) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ZStack {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(.quaternary.opacity(0.35))
                if let image = thumbs.images[asset.thumbnailAssetId ?? asset.id] {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: asset.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                        .foregroundStyle(.tint)
                }
            }
            .aspectRatio(1, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            Text(asset.fileName)
                .font(.caption2)
                .lineLimit(1)
        }
    }


    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            async let assetsTask = client.libraryAssets(filter: "all", authorId: author.id, limit: 1000)
            async let rootsTask = client.libraryRoots()
            assets = try await assetsTask
            roots = (try? await rootsTask) ?? []
            if let resolved = pmlResolveAuthorFolder(assets: assets, roots: roots, authorName: info.name) {
                folderRoot = resolved.root
                folderPath = resolved.path
                await loadAuthorFolder()
            }
            if let authors = try? await client.libraryAuthors(filter: "all"),
               let latest = authors.first(where: { $0.id == author.id }) {
                displayAuthor = latest
            }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }

    private func loadAuthorFolder() async {
        guard let client = model.apiClient, let folderRoot, !folderPath.isEmpty else { return }
        folderLoading = true
        do {
            let page = try await client.remoteChildren(rootId: folderRoot.id, path: folderPath, sort: sort.rawValue)
            folderEntries = page.items
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        folderLoading = false
    }

    private func open(_ entry: RemoteBrowserEntry) async {
        guard let assetId = entry.assetId,
              let source = await model.playbackSource(assetId: assetId, remotePath: entry.path) else { return }
        preview = PreviewItem(
            url: source.url,
            isVideo: entry.isVideo,
            assetId: assetId,
            title: entry.name,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: entry.path
        )
    }

    private var authorColumns: [GridItem] {
        Array(repeating: GridItem(.flexible(), spacing: 6), count: layout == .grid ? 3 : 2)
    }

    @ViewBuilder
    private func folderContent(root: LibraryRoot) -> some View {
        if folderEntries.isEmpty {
            Text("无文件").foregroundStyle(.secondary)
        } else if layout == .list {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(folderEntries) { entry in
                    if entry.isDirectory {
                        NavigationLink {
                            DirectoryView(root: root, path: entry.path)
                        } label: {
                            authorEntryRow(entry)
                        }
                        .id("dir-\(entry.path)")
                        .buttonStyle(.plain)
                    } else {
                        Button {
                            Task { await open(entry) }
                        } label: {
                            authorEntryRow(entry)
                        }
                        .id("file-\(entry.path)")
                        .buttonStyle(.plain)
                    }
                }
            }
        } else {
            LazyVGrid(columns: authorColumns, spacing: 6) {
                ForEach(folderEntries) { entry in
                    if entry.isDirectory {
                        NavigationLink {
                            DirectoryView(root: root, path: entry.path)
                        } label: {
                            authorEntryCell(entry)
                        }
                        .id("dir-\(entry.path)")
                        .buttonStyle(.plain)
                    } else {
                        Button {
                            Task { await open(entry) }
                        } label: {
                            authorEntryCell(entry)
                        }
                        .id("file-\(entry.path)")
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private func authorEntryCell(_ entry: RemoteBrowserEntry) -> some View {
        let thumbId = entry.assetId ?? entry.coverAssetId
        return VStack(alignment: .leading, spacing: 4) {
            Color.clear
                .aspectRatio(layout == .wall ? 1 : 0.8, contentMode: .fit)
                .overlay {
                    ZStack {
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .fill(.quaternary.opacity(0.3))
                        AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                            Image(systemName: entry.isDirectory ? "folder.fill" : (entry.isVideo ? "play.rectangle.fill" : "photo.fill"))
                                .font(.title3)
                                .foregroundStyle(entry.isDirectory ? Color.accentColor : Color.secondary)
                        }
                    }
                    .allowsHitTesting(false)
                }
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .clipped()
            Text(pmlDisplayName(entry.name))
                .font(.caption2)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .clipped()
        .contentShape(Rectangle())
    }

    private func authorEntryRow(_ entry: RemoteBrowserEntry) -> some View {
        HStack(spacing: 12) {
            Image(systemName: entry.isDirectory ? "folder.fill" : (entry.isVideo ? "play.rectangle.fill" : "photo.fill"))
                .foregroundStyle(entry.isDirectory ? Color.accentColor : Color.secondary)
            Text(pmlDisplayName(entry.name)).lineLimit(1)
            Spacer()
            if entry.isDirectory {
                Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
            }
        }
        .padding(.vertical, 8)
        .contentShape(Rectangle())
    }
    private func open(_ asset: LibraryAsset) async {
        guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else { return }
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
