import SwiftUI

// MARK: - 四域入口

struct FourDomainHomeView: View {
    @EnvironmentObject private var model: AppModel
    @State private var summaries: [DomainSummary] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var scanning: String?
    @State private var cleanMessage = ""

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if summaries.isEmpty && loading {
                        ProgressView().frame(maxWidth: .infinity)
                    } else {
                        ForEach(summaries) { summary in
                            NavigationLink {
                                destination(for: summary.domain)
                            } label: {
                                DomainSummaryRow(summary: summary, scanning: scanning == summary.domain)
                            }
                            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                Button {
                                    Task { await scan(summary.domain) }
                                } label: {
                                    Label("扫描", systemImage: "arrow.clockwise")
                                }
                                .tint(.blue)
                                Button {
                                    Task { await clean(summary.domain) }
                                } label: {
                                    Label("清理", systemImage: "trash")
                                }
                                .tint(.orange)
                            }
                        }
                    }
                } header: {
                    Text("四大区域")
                } footer: {
                    Text("每个区域独立扫描、独立清理。最新添加是物理临时区；待整理按第一层【Pending】目录分组。")
                }

                if !cleanMessage.isEmpty {
                    Section { Text(cleanMessage).font(.footnote).foregroundStyle(.secondary) }
                }

                Section("快捷操作") {
                    Button {
                        Task { await scanAll() }
                    } label: {
                        Label(scanning == nil ? "扫描全部区域" : "正在扫描…", systemImage: "arrow.clockwise.circle")
                    }
                    .disabled(scanning != nil)
                    NavigationLink {
                        MoveConflictListView()
                    } label: {
                        Label("同名冲突", systemImage: "exclamationmark.triangle")
                    }
                }

                if !errorText.isEmpty {
                    Section {
                        Text(errorText)
                            .font(.footnote)
                            .foregroundStyle(.red)
                        Button("重试") { Task { await load() } }
                    }
                }
            }
            .navigationTitle("媒体库")
            .refreshable { await load() }
            .task { await load() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task { await load() }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(loading)
                }
            }
        }
    }

    @ViewBuilder
    private func destination(for domain: String) -> some View {
        switch domain {
        case "pending":
            PendingProjectsView()
        default:
            DomainBrowserView(
                domain: domain,
                title: domainTitle(domain)
            )
        }
    }

    private func domainTitle(_ domain: String) -> String {
        summaries.first(where: { $0.domain == domain })?.title ?? domain
    }

    private func load() async {
        guard let client = model.apiClient else {
            errorText = "当前不是服务器模式"
            return
        }
        loading = true
        errorText = ""
        do {
            summaries = try await client.domainSummaries()
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
        loading = false
    }

    private func scan(_ domain: String) async {
        guard let client = model.apiClient, scanning == nil else { return }
        scanning = domain
        do {
            let result = try await client.scanDomain(domain)
            cleanMessage = "\(domain): 扫描 \(result.scanned)，新增 \(result.added)，缺失 \(result.missing)，清理 \(result.deleted)"
            await load()
        } catch {
            errorText = error.localizedDescription
        }
        scanning = nil
    }

    private func scanAll() async {
        for domain in ["recent", "pending", "vault", "comics"] {
            await scan(domain)
        }
    }

    private func clean(_ domain: String) async {
        guard let client = model.apiClient else { return }
        do {
            let preview = try await client.cleanDomain(domain, dryRun: true)
            guard preview.count > 0 else {
                cleanMessage = "\(domain): 没有可清理的 missing 记录"
                return
            }
            let result = try await client.cleanDomain(domain, dryRun: false)
            cleanMessage = "\(domain): 已清理 \(result.count) 条 missing"
            await load()
        } catch {
            errorText = error.localizedDescription
        }
    }
}

private struct DomainSummaryRow: View {
    let summary: DomainSummary
    let scanning: Bool

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: summary.symbol)
                .font(.system(size: 22, weight: .semibold))
                .foregroundStyle(.tint)
                .frame(width: 42, height: 42)
                .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(summary.title).font(.headline)
                    if scanning {
                        ProgressView().controlSize(.small)
                    }
                }
                Text(summary.rootPath ?? "未配置")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                HStack(spacing: 10) {
                    Text("\(summary.present) 正常")
                    if summary.missing > 0 {
                        Text("\(summary.missing) 缺失").foregroundStyle(.orange)
                    }
                    if let status = summary.lastScanStatus {
                        Text(status == "success" ? "已扫描" : status).foregroundStyle(.secondary)
                    }
                }
                .font(.caption2)
            }
            Spacer()
        }
        .padding(.vertical, 4)
    }
}


// MARK: - 域目录浏览

struct DomainBrowserView: View {
    let domain: String
    let title: String
    var path: String?

    @EnvironmentObject private var model: AppModel
    @State private var children: [DomainChild] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var videoPreview: DomainPreview?
    @State private var imagePreview: DomainImagePreview?
    @State private var comicTarget: ComicArchiveTarget?

    var body: some View {
        List {
            if !errorText.isEmpty {
                Section {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                    Button("重试") { Task { await load() } }
                }
            }
            if loading && children.isEmpty {
                ProgressView().frame(maxWidth: .infinity)
            }
            ForEach(children) { child in
                if child.isDirectory {
                    NavigationLink {
                        DomainBrowserView(
                            domain: domain,
                            title: child.name,
                            path: child.path
                        )
                    } label: {
                        DomainFolderRow(child: child)
                    }
                } else {
                    Button {
                        Task { await open(child) }
                    } label: {
                        DomainFileRow(child: child)
                    }
                    .buttonStyle(.plain)
                }
            }
            if children.isEmpty && !loading && errorText.isEmpty {
                ContentUnavailableView("空目录", systemImage: "folder")
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .fullScreenCover(item: $videoPreview) { preview in
            GestureVideoPlayerSheet(
                url: preview.url,
                title: preview.title,
                assetId: preview.assetId,
                fallbackURL: preview.fallbackURL,
                sourceLabel: preview.sourceLabel
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $imagePreview) { preview in
            AuthenticatedImageViewer(url: preview.url, title: preview.title)
                .environmentObject(model)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target)
                .environmentObject(model)
        }
    }

    private func load() async {
        guard let client = model.apiClient else {
            errorText = "当前不是服务器模式"
            return
        }
        loading = true
        errorText = ""
        do {
            children = try await client.domainChildren(domain: domain, path: path)
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loading = false
    }

    private func open(_ child: DomainChild) async {
        guard let assetId = child.assetId else { return }
        if child.isComic {
            comicTarget = ComicArchiveTarget(id: assetId, title: child.name)
            return
        }
        do {
            guard let source = await model.playbackSource(assetId: assetId, remotePath: child.path) else {
                errorText = "无法获取播放地址"
                return
            }
            if child.isVideo {
                videoPreview = DomainPreview(
                    url: source.url,
                    title: child.name,
                    assetId: assetId,
                    fallbackURL: source.fallbackURL,
                    sourceLabel: source.label
                )
            } else if child.isImage {
                imagePreview = DomainImagePreview(url: source.url, title: child.name)
            }
        } catch {
            errorText = error.localizedDescription
        }
    }
}

private struct DomainFolderRow: View {
    let child: DomainChild

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "folder.fill")
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(.tint)
                .frame(width: 44, height: 44)
                .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            VStack(alignment: .leading, spacing: 3) {
                Text(child.name).font(.subheadline.weight(.medium)).lineLimit(2)
                Text("\(child.childCount) 个文件").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(.vertical, 2)
    }
}

private struct DomainFileRow: View {
    let child: DomainChild
    @EnvironmentObject private var model: AppModel
    @State private var image: UIImage?

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if let image {
                    Image(uiImage: image).resizable().scaledToFill()
                } else {
                    Image(systemName: child.isVideo ? "play.rectangle.fill" : child.isComic ? "book.closed.fill" : "photo.fill")
                        .font(.system(size: 20, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: 52, height: 52)
            .background(Color.secondary.opacity(0.1), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))

            VStack(alignment: .leading, spacing: 3) {
                Text(child.name).font(.subheadline.weight(.medium)).lineLimit(2)
                Text("\(child.mediaType ?? "file") · \(pmlFormatBytes(child.sizeBytes))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if child.isVideo {
                Image(systemName: "play.circle.fill").font(.title3).foregroundStyle(.tint)
            }
        }
        .padding(.vertical, 2)
        .task(id: child.thumbnailAssetId ?? child.assetId) {
            if let assetId = child.thumbnailAssetId ?? child.assetId {
                image = await ThumbnailService.shared.image(assetId: assetId, client: model.apiClient)
            }
        }
    }
}

// MARK: - 域资产列表

private struct DomainPreview: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
    let assetId: String?
    let fallbackURL: URL?
    let sourceLabel: String?
}

private struct DomainImagePreview: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
}

struct DomainAssetListView: View {
    let domain: String
    let title: String
    var allowsSearch: Bool = false

    @EnvironmentObject private var model: AppModel
    @State private var assets: [DomainAsset] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var query = ""
    @State private var videoPreview: DomainPreview?
    @State private var imagePreview: DomainImagePreview?
    @State private var comicTarget: ComicArchiveTarget?
    @State private var movingAsset: String?

    var body: some View {
        List {
            if !errorText.isEmpty {
                Section {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                    Button("重试") { Task { await load() } }
                }
            }
            if loading && assets.isEmpty {
                ProgressView().frame(maxWidth: .infinity)
            }
            ForEach(assets) { asset in
                Button {
                    Task { await open(asset) }
                } label: {
                    DomainAssetRow(asset: asset)
                }
                .buttonStyle(.plain)
                .disabled(movingAsset != nil)
            }
        }
        .navigationTitle(title)
        .searchable(text: $query, prompt: "搜索文件名")
        .onChange(of: query) { _, _ in
            guard allowsSearch else { return }
            Task {
                try? await Task.sleep(nanoseconds: 250_000_000)
                await load()
            }
        }
        .refreshable { await load() }
        .task { await load() }
        .fullScreenCover(item: $videoPreview) { preview in
            GestureVideoPlayerSheet(
                url: preview.url,
                title: preview.title,
                assetId: preview.assetId,
                fallbackURL: preview.fallbackURL,
                sourceLabel: preview.sourceLabel
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $imagePreview) { preview in
            AuthenticatedImageViewer(url: preview.url, title: preview.title)
                .environmentObject(model)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target)
                .environmentObject(model)
        }
    }

    private func load() async {
        guard let client = model.apiClient else {
            errorText = "当前不是服务器模式"
            return
        }
        loading = true
        errorText = ""
        do {
            assets = try await client.domainAssets(
                domain: domain,
                query: allowsSearch && !query.isEmpty ? query : nil,
                limit: 300
            )
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
        loading = false
    }

    private func open(_ asset: DomainAsset) async {
        guard let client = model.apiClient else { return }
        if asset.isComic {
            comicTarget = ComicArchiveTarget(id: asset.id, title: asset.fileName)
            return
        }
        do {
            guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else {
                errorText = "无法获取播放地址"
                return
            }
            if asset.isVideo {
                videoPreview = DomainPreview(
                    url: source.url,
                    title: asset.fileName,
                    assetId: asset.id,
                    fallbackURL: source.fallbackURL,
                    sourceLabel: source.label
                )
            } else if asset.isImage {
                imagePreview = DomainImagePreview(url: source.url, title: asset.fileName)
            }
        } catch {
            errorText = error.localizedDescription
        }
        _ = client
    }
}

private struct DomainAssetRow: View {
    let asset: DomainAsset
    @EnvironmentObject private var model: AppModel
    @State private var image: UIImage?

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if let image {
                    Image(uiImage: image).resizable().scaledToFill()
                } else {
                    Image(systemName: asset.symbol)
                        .font(.system(size: 20, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: 58, height: 58)
            .background(Color.secondary.opacity(0.1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            VStack(alignment: .leading, spacing: 4) {
                Text(asset.fileName).font(.subheadline.weight(.medium)).lineLimit(2)
                Text("\(asset.mediaType) · \(pmlFormatBytes(asset.sizeBytes))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if asset.favorite {
                    Label("收藏", systemImage: "heart.fill").font(.caption2).foregroundStyle(.pink)
                }
            }
            Spacer()
            if asset.isVideo {
                Image(systemName: "play.circle.fill").font(.title3).foregroundStyle(.tint)
            }
        }
        .padding(.vertical, 2)
        .task(id: asset.thumbnailAssetId ?? asset.id) {
            image = await ThumbnailService.shared.image(
                assetId: asset.thumbnailAssetId ?? asset.id,
                client: model.apiClient
            )
        }
    }
}

private struct AuthenticatedImageViewer: View {
    let url: URL
    let title: String
    @Environment(\.dismiss) private var dismiss
    @State private var image: UIImage?
    @State private var failed = false

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()
                if let image {
                    Image(uiImage: image).resizable().scaledToFit()
                } else if failed {
                    Label("图片加载失败", systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.white)
                } else {
                    ProgressView().tint(.white)
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
            .task {
                image = await ImageCache.shared.image(for: url, maxPixel: 2400)
                failed = image == nil
            }
        }
    }
}

// MARK: - 待整理项目

struct PendingProjectsView: View {
    @EnvironmentObject private var model: AppModel
    @State private var projects: [PendingProject] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var showCreate = false

    var body: some View {
        List {
            if loading && projects.isEmpty {
                ProgressView().frame(maxWidth: .infinity)
            }
            if !errorText.isEmpty {
                Section {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                }
            }
            ForEach(projects) { project in
                NavigationLink {
                    PendingProjectDetailView(project: project)
                } label: {
                    PendingProjectRow(project: project)
                }
            }
            if projects.isEmpty && !loading {
                ContentUnavailableView("没有待整理项目", systemImage: "tray", description: Text("从最近添加中选择文件生成项目"))
            }
        }
        .navigationTitle("待整理")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showCreate = true
                } label: {
                    Image(systemName: "plus")
                }
            }
        }
        .sheet(isPresented: $showCreate) {
            CreatePendingProjectView {
                await load()
            }
            .environmentObject(model)
        }
        .refreshable { await load() }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.apiClient else { errorText = "当前不是服务器模式"; return }
        loading = true
        errorText = ""
        do {
            projects = try await client.pendingProjects()
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
        loading = false
    }
}

private struct PendingProjectRow: View {
    let project: PendingProject

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(project.name).font(.headline)
                if project.isLoose {
                    Text("散装").font(.caption2).padding(.horizontal, 6).padding(.vertical, 2)
                        .background(Color.orange.opacity(0.15), in: Capsule())
                }
                Spacer()
                if project.conflictCount > 0 {
                    Label("\(project.conflictCount)", systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(.red)
                }
            }
            Text(project.sourcePath).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            HStack(spacing: 12) {
                Label("\(project.totalCount)", systemImage: "doc.on.doc")
                Label("\(project.triageCount)", systemImage: "questionmark.circle")
                Label("\(project.readyCount)", systemImage: "checkmark.circle")
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }
}

struct PendingProjectDetailView: View {
    let project: PendingProject
    @EnvironmentObject private var model: AppModel
    @State private var items: [PendingDomainItem] = []
    @State private var meta: DomainMeta?
    @State private var loading = false
    @State private var errorText = ""
    @State private var moving = false
    @State private var summary = ""
    @State private var preview: DomainPreview?

    var body: some View {
        List {
            Section {
                HStack {
                    Label("\(project.totalCount) 文件", systemImage: "doc.on.doc")
                    Spacer()
                    Text("待指定 \(project.triageCount)").foregroundStyle(.orange)
                    Text("已指定 \(project.readyCount)").foregroundStyle(.green)
                }
                .font(.caption)
                Text(project.sourcePath).font(.caption2).foregroundStyle(.secondary)
            } header: {
                Text(project.name)
            }

            if !errorText.isEmpty {
                Section { Text(errorText).font(.footnote).foregroundStyle(.red) }
            }
            if !summary.isEmpty {
                Section { Text(summary).font(.footnote).foregroundStyle(.secondary) }
            }

            Section("文件") {
                if loading && items.isEmpty {
                    ProgressView().frame(maxWidth: .infinity)
                }
                ForEach(items) { item in
                    PendingDomainItemRow(item: item, meta: meta) { categoryId, authorId in
                        Task { await assign(item, categoryId: categoryId, authorId: authorId) }
                    } onPreview: {
                        Task { await openPreview(item) }
                    }
                }
            }

            Section {
                Button {
                    Task { await moveReady() }
                } label: {
                    if moving {
                        HStack { ProgressView(); Text("正在移动…") }
                    } else {
                        Label("移动已指定文件", systemImage: "arrow.right.circle.fill")
                    }
                }
                .disabled(moving || items.filter(\.isReady).isEmpty)
            } footer: {
                Text("同名目标会报警并跳过，不覆盖、不改名。")
            }
        }
        .navigationTitle(project.name)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(
                url: item.url,
                title: item.title,
                assetId: item.assetId,
                fallbackURL: item.fallbackURL,
                sourceLabel: item.sourceLabel
            )
            .environmentObject(model)
        }
    }

    private func load() async {
        guard let client = model.apiClient else { errorText = "当前不是服务器模式"; return }
        loading = true
        errorText = ""
        do {
            async let loadedItems = client.pendingProjectItems(projectId: project.id)
            async let loadedMeta = client.domainMeta()
            items = try await loadedItems
            meta = try await loadedMeta
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
        loading = false
    }

    private func assign(_ item: PendingDomainItem, categoryId: String?, authorId: String?) async {
        guard let client = model.apiClient else { return }
        do {
            try await client.assignPending(assetId: item.assetId, categoryId: categoryId, authorId: authorId)
            await load()
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func moveReady() async {
        guard let client = model.apiClient, !moving else { return }
        moving = true
        summary = ""
        var moved = 0
        var conflicts = 0
        var failed = 0
        for item in items where item.isReady {
            do {
                let result = try await client.movePendingDomainItem(assetId: item.assetId)
                if result.success { moved += 1 }
                else if result.status == "SKIPPED_EXIST" { conflicts += 1 }
                else { failed += 1 }
            } catch {
                failed += 1
            }
        }
        summary = "移动完成：成功 \(moved)，冲突跳过 \(conflicts)，失败 \(failed)"
        moving = false
        await load()
    }

    private func openPreview(_ item: PendingDomainItem) async {
        guard let source = await model.playbackSource(assetId: item.assetId, remotePath: item.remotePath) else {
            errorText = "无法获取播放地址"
            return
        }
        preview = DomainPreview(
            url: source.url,
            title: item.fileName,
            assetId: item.assetId,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label
        )
    }
}

private struct PendingDomainItemRow: View {
    let item: PendingDomainItem
    let meta: DomainMeta?
    let onAssign: (String?, String?) -> Void
    let onPreview: () -> Void

    private var categoryName: String? {
        if let manual = item.manualCategoryId,
           let name = meta?.categories.first(where: { $0.id == manual })?.name {
            return name
        }
        return item.recommendedCategoryName
    }

    private var authorName: String? {
        if let manual = item.manualAuthorId,
           let name = meta?.authors.first(where: { $0.id == manual })?.name {
            return name
        }
        return item.recommendedAuthorName
    }

    private var currentCategoryId: String? { item.manualCategoryId ?? item.recommendedCategoryId }
    private var currentAuthorId: String? { item.manualAuthorId ?? item.recommendedAuthorId }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button(action: onPreview) {
                HStack(spacing: 10) {
                    Image(systemName: item.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                        .foregroundStyle(.tint)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(item.fileName).font(.subheadline.weight(.medium)).lineLimit(2)
                        Text("\(item.mediaType) · \(pmlFormatBytes(item.sizeBytes))")
                            .font(.caption2).foregroundStyle(.secondary)
                    }
                    Spacer()
                }
            }
            .buttonStyle(.plain)

            HStack(spacing: 8) {
                Menu {
                    Button("不指定分类") { onAssign(nil, currentAuthorId) }
                    Divider()
                    ForEach(meta?.categories ?? []) { category in
                        Button(category.name) { onAssign(category.id, currentAuthorId) }
                    }
                } label: {
                    PendingChip(title: categoryName ?? "选择分类", color: .blue)
                }

                Menu {
                    Button("无作者") { onAssign(currentCategoryId, nil) }
                    Divider()
                    ForEach(meta?.authors ?? []) { author in
                        Button(author.name) { onAssign(currentCategoryId, author.id) }
                    }
                } label: {
                    PendingChip(title: authorName ?? "选择作者", color: .purple)
                }

                if let confidence = item.confidence {
                    Text("\(Int(confidence * 100))%")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if item.assignmentStatus == "ready" {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                }
            }

            if let reason = item.reason {
                Text(reason).font(.caption2).foregroundStyle(.secondary)
            }
            if let conflict = item.conflictReason {
                Label(conflict, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption2).foregroundStyle(.red)
            }
        }
        .padding(.vertical, 4)
    }
}

private struct PendingChip: View {
    let title: String
    let color: Color

    var body: some View {
        Text(title)
            .font(.caption)
            .lineLimit(1)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(color.opacity(0.14), in: Capsule())
            .foregroundStyle(color)
    }
}

struct CreatePendingProjectView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let onCreated: () async -> Void

    @State private var assets: [DomainAsset] = []
    @State private var selected = Set<String>()
    @State private var name = ""
    @State private var loading = true
    @State private var creating = false
    @State private var errorText = ""

    var body: some View {
        NavigationStack {
            List {
                Section {
                    TextField("项目名，例如 Pending7", text: $name)
                        .textInputAutocapitalization(.never)
                    Text("会创建【\(name.isEmpty ? "项目名" : name)】目录并物理移动所选文件。项目名创建后不可修改。")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Section("从最新添加选择") {
                    if loading {
                        ProgressView()
                    }
                    ForEach(assets) { asset in
                        Button {
                            if selected.contains(asset.id) { selected.remove(asset.id) }
                            else { selected.insert(asset.id) }
                        } label: {
                            HStack {
                                Image(systemName: selected.contains(asset.id) ? "checkmark.circle.fill" : "circle")
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(asset.fileName).font(.subheadline).lineLimit(2)
                                    Text(pmlFormatBytes(asset.sizeBytes)).font(.caption2).foregroundStyle(.secondary)
                                }
                                Spacer()
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                if !errorText.isEmpty {
                    Section { Text(errorText).font(.footnote).foregroundStyle(.red) }
                }
            }
            .navigationTitle("生成待整理项目")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("取消") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button(creating ? "移动中…" : "创建") {
                        Task { await create() }
                    }
                    .disabled(creating || name.trimmingCharacters(in: .whitespaces).isEmpty || selected.isEmpty)
                }
            }
            .task { await load() }
        }
    }

    private func load() async {
        guard let client = model.apiClient else { loading = false; return }
        do {
            assets = try await client.domainAssets(domain: "recent", limit: 200)
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }

    private func create() async {
        guard let client = model.apiClient, !creating else { return }
        creating = true
        errorText = ""
        do {
            _ = try await client.createPendingProject(name: name, assetIds: Array(selected))
            await onCreated()
            dismiss()
        } catch {
            errorText = error.localizedDescription
        }
        creating = false
    }
}

struct MoveConflictListView: View {
    @EnvironmentObject private var model: AppModel
    @State private var conflicts: [MoveConflict] = []
    @State private var loading = false
    @State private var errorText = ""

    var body: some View {
        List {
            if loading && conflicts.isEmpty { ProgressView() }
            if !errorText.isEmpty { Text(errorText).foregroundStyle(.red) }
            ForEach(conflicts) { conflict in
                VStack(alignment: .leading, spacing: 4) {
                    Text(conflict.kind).font(.headline)
                    Text(conflict.sourcePath).font(.caption).lineLimit(1)
                    Text("→ \(conflict.targetPath)").font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            if conflicts.isEmpty && !loading {
                ContentUnavailableView("没有同名冲突", systemImage: "checkmark.circle")
            }
        }
        .navigationTitle("同名冲突")
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        do { conflicts = try await client.moveConflicts() }
        catch { if !pmlIsCancellation(error) { errorText = error.localizedDescription } }
        loading = false
    }
}
