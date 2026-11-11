import PhotosUI
import SwiftUI
import UIKit

/// 目录路由用值类型，避免 Lazy 容器复用 NavigationLink 时打开错目录。
struct DirectoryRoute: Hashable {
    let root: LibraryRoot
    let path: String

    static func == (lhs: DirectoryRoute, rhs: DirectoryRoute) -> Bool {
        lhs.root.id == rhs.root.id && lhs.path == rhs.path
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(root.id)
        hasher.combine(path)
    }
}

/// 目录浏览：列表 / 网格 / 照片墙三视图，排序、搜索、收藏与缩略图按需生成。
struct DirectoryView: View {
    let root: LibraryRoot
    let path: String
    let domain: String?
    let titleOverride: String?

    init(
        root: LibraryRoot,
        path: String,
        domain: String? = nil,
        titleOverride: String? = nil
    ) {
        self.root = root
        self.path = path
        self.domain = domain
        self.titleOverride = titleOverride
    }

    @EnvironmentObject private var model: AppModel

    @State private var entries: [RemoteBrowserEntry] = []
    @State private var nextCursor: String?
    @State private var loading = true
    @State private var loadingMore = false
    @State private var errorText = ""
    @State private var noticeText = ""
    @State private var autoRefreshed = false
    @State private var coverItem: PhotosPickerItem?
    @State private var showCoverPicker = false

    @State private var layout: BrowseLayout = .list
    @State private var sort: BrowseSort = .name
    @State private var searchText = ""
    @State private var refreshTurns = 0
    @State private var favorites: Set<String> = []

    @State private var preview: PreviewItem?
    @State private var videoPlaylist: [VideoPlaylistItem] = []
    @State private var videoPlaylistIndex: Int?
    @State private var imageContext: ImageViewerContext?
    @State private var comicTarget: ComicArchiveTarget?
    @State private var folderComic: ComicFolderTarget?
    @State private var showRemoteSearch = false

    @ObservedObject private var thumbs = ThumbnailService.shared

    /// Sort locally so name order follows the system locale (拼音/自然数字) and
    /// switching sort takes effect immediately, instead of waiting for a reload.
    private var sortedEntries: [RemoteBrowserEntry] {
        entries.sorted { left, right in
            if left.isDirectory != right.isDirectory { return left.isDirectory }
            switch sort {
            case .name:
                break
            case .newest:
                let l = modifiedDate(left.modifiedAt) ?? .distantPast
                let r = modifiedDate(right.modifiedAt) ?? .distantPast
                if l != r { return l > r }
            case .size:
                if left.sizeBytes != right.sizeBytes { return left.sizeBytes > right.sizeBytes }
            }
            let ordered = left.name.localizedStandardCompare(right.name)
            if ordered == .orderedSame {
                return left.path < right.path
            }
            return ordered == .orderedAscending
        }
    }

    private var visibleEntries: [RemoteBrowserEntry] {
        let sorted = sortedEntries
        guard !searchText.isEmpty else { return sorted }
        return sorted.filter { $0.name.localizedCaseInsensitiveContains(searchText) }
    }

    private var imageEntries: [RemoteBrowserEntry] {
        visibleEntries.filter { $0.isImage }
    }

    var body: some View {
        ScrollView {
            folderHeader
            if layout == .list {
                LazyVStack(spacing: 0) {
                    contentRows
                }
                .id(sort)
            } else {
                LazyVGrid(columns: gridColumns, spacing: 6) {
                    contentCells
                }
                .id(sort)
                .padding(.horizontal, 6)
                .padding(.top, 6)
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "搜索当前目录")
        .refreshable { await reload(force: true) }
        .task { await bootstrap() }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                HStack(spacing: 14) {
                    if root.kind == "comics" && !entries.isEmpty {
                        Button {
                            folderComic = ComicFolderTarget(root: root, path: path, startPath: nil)
                        } label: {
                            Image(systemName: "book")
                        }
                    }
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
                        Divider()
                        Button {
                            showCoverPicker = true
                        } label: {
                            Label("上传目录封面", systemImage: "photo.badge.plus")
                        }
                    } label: {
                        Image(systemName: layout.symbol)
                    }
                    .onChange(of: layout) { _, newValue in
                        BrowsePreferences.shared.setLayout(newValue, rootId: root.id, path: path)
                    }
                    .onChange(of: sort) { _, newValue in
                        BrowsePreferences.shared.setSort(newValue, rootId: root.id, path: path)
                        Task { await reload(force: false) }
                    }
                    .photosPicker(isPresented: $showCoverPicker, selection: $coverItem, matching: .images)
                    .onChange(of: coverItem) { _, item in
                        guard let item else { return }
                        Task { await uploadDirectoryCover(item) }
                    }

                    if domain == nil {
                        Button {
                            showRemoteSearch = true
                        } label: {
                            Image(systemName: "magnifyingglass")
                        }
                    }

                    Button {
                        refreshTurns += 1
                        Task { await reload(force: true) }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                            .rotationEffect(.degrees(Double(refreshTurns) * 360))
                            .animation(.easeInOut(duration: 0.6), value: refreshTurns)
                    }
                    .disabled(loading)
                }
            }
        }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(
                url: item.url,
                title: item.title,
                assetId: item.assetId,
                startAt: item.startAt,
                playlist: videoPlaylist,
                playlistIndex: videoPlaylistIndex,
                fallbackURL: item.fallbackURL,
                sourceLabel: item.sourceLabel,
                remotePath: item.remotePath
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $imageContext) { context in
            ImageViewer(context: context)
                .environmentObject(model)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target)
                .environmentObject(model)
        }
        .fullScreenCover(item: $folderComic) { target in
            ComicFolderReaderView(target: target)
                .environmentObject(model)
        }

        .sheet(isPresented: $showRemoteSearch) {
            RemoteSearchView(root: root, parentPath: path)
                .environmentObject(model)
        }
    }

    private var folderHeader: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 14) {
                folderCover
                VStack(alignment: .leading, spacing: 6) {
                    Text(title)
                        .font(.title3.weight(.semibold))
                        .lineLimit(2)
                    Text(folderMeta)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button {
                        Task { await playFolder() }
                    } label: {
                        Label(resumeVideo == nil ? "播放" : "继续播放", systemImage: "play.fill")
                            .font(.subheadline.weight(.semibold))
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(playableVideos.isEmpty)
                }
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 4)
    }

    private var folderCover: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(.quaternary.opacity(0.35))
            if let id = folderCoverId, let image = thumbs.images[id] {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                Image(systemName: root.kind == "comics" ? "book.closed.fill" : "folder.fill")
                    .font(.title2)
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: 92, height: 128)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        .task(id: folderCoverId) {
            guard let id = folderCoverId else { return }
            await thumbs.image(assetId: id, client: model.apiClient)
        }
    }

    private var playableVideos: [RemoteBrowserEntry] {
        visibleEntries.filter { $0.isVideo && $0.assetId != nil }
    }

    private var resumeVideo: RemoteBrowserEntry? {
        playableVideos.filter { ($0.progressSeconds ?? 0) > 2 }
            .max { ($0.progressSeconds ?? 0) < ($1.progressSeconds ?? 0) }
    }

    private var folderCoverId: String? {
        visibleEntries.compactMap(\.coverAssetId).first
            ?? visibleEntries.first(where: { !$0.isDirectory })?.assetId
    }

    private var folderMeta: String {
        let folders = visibleEntries.filter(\.isDirectory).count
        let files = visibleEntries.filter { !$0.isDirectory }.count
        let time = visibleEntries.compactMap { modifiedDate($0.modifiedAt) }.max()
        var parts = ["\(files) 个文件"]
        if folders > 0 { parts.append("\(folders) 个文件夹") }
        if let time { parts.append(DirectoryView.modifiedDisplay.string(from: time)) }
        return parts.joined(separator: " · ")
    }

    private func playFolder() async {
        guard let entry = resumeVideo ?? playableVideos.first else { return }
        await open(entry)
    }

    private var title: String {
        if let titleOverride, !titleOverride.isEmpty { return titleOverride }
        return path.split(separator: "/").last.map(String.init) ?? root.kindLabel
    }

    private var gridColumns: [GridItem] {
        let count = layout == .grid ? 3 : 2
        return Array(repeating: GridItem(.flexible(), spacing: 6), count: count)
    }

    private var hintText: String {
        if root.kind == "media" {
            return "「综合媒体」只索引图片和视频。如果这里是漫画或 CBZ/ZIP，请左滑删除该根目录，再用「漫画」类型重新添加。"
        }
        return "「漫画」类型只显示图片与 CBZ/ZIP。如果这里是视频，请用「综合媒体」类型重新添加根目录。"
    }

    // MARK: - 列表内容

    @ViewBuilder
    private var contentRows: some View {
        if loading && entries.isEmpty {
            HStack(spacing: 10) {
                ProgressView()
                Text("加载中…").foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .center)
            .padding(40)
        }

        ForEach(visibleEntries) { entry in
            row(for: entry)
        }

        listFooter
    }

    @ViewBuilder
    private var contentCells: some View {
        ForEach(visibleEntries) { entry in
            cell(for: entry)
        }

        if let nextCursor {
            Button {
                Task { await loadMore(cursor: nextCursor) }
            } label: {
                HStack(spacing: 8) {
                    if loadingMore { ProgressView() }
                    Text(loadingMore ? "加载中…" : "加载更多")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 14)
            }
            .gridCellColumns(gridColumns.count)
        }

        if !errorText.isEmpty {
            Text(errorText)
                .font(.footnote)
                .foregroundStyle(.red)
                .gridCellColumns(gridColumns.count)
        }

        if !noticeText.isEmpty {
            Text(noticeText)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .gridCellColumns(gridColumns.count)
        }

        if !loading && entries.isEmpty && errorText.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("这个目录里没有可显示的媒体文件").font(.callout)
                Text(hintText).font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
            .gridCellColumns(gridColumns.count)
        }
    }

    @ViewBuilder
    private var listFooter: some View {
        if let nextCursor {
            Button {
                Task { await loadMore(cursor: nextCursor) }
            } label: {
                HStack(spacing: 8) {
                    if loadingMore { ProgressView() }
                    Text(loadingMore ? "加载中…" : "加载更多")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 14)
            }
        }

        if !errorText.isEmpty {
            Text(errorText).font(.footnote).foregroundStyle(.red).padding()
        }

        if !noticeText.isEmpty {
            Text(noticeText).font(.footnote).foregroundStyle(.secondary).padding()
        }

        if !loading && entries.isEmpty && errorText.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("这个目录里没有可显示的媒体文件").font(.callout)
                Text(hintText).font(.caption).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(20)
        }
    }

    // MARK: - 单元格

    @ViewBuilder
    private func row(for entry: RemoteBrowserEntry) -> some View {
        let content = HStack(spacing: 12) {
            thumbnail(entry, size: 50)
            VStack(alignment: .leading, spacing: 3) {
                Text(entry.name).lineLimit(1)
                Text(subtitle(entry))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 6)
            if entry.isDirectory {
                Image(systemName: "chevron.right")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            if let assetId = entry.assetId, favorites.contains(assetId) {
                Image(systemName: "heart.fill")
                    .font(.caption)
                    .foregroundStyle(.pink)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 7)
        .contentShape(Rectangle())

        if entry.isDirectory {
            NavigationLink {
                DirectoryView(root: root, path: entry.path, domain: domain)
            } label: {
                content
            }
            .id("dir-\(entry.path)")
            .buttonStyle(.plain)
            .contextMenu { contextMenu(for: entry) }
        } else {
            Button {
                Task { await open(entry) }
            } label: {
                content
            }
            .buttonStyle(.plain)
            .contextMenu { contextMenu(for: entry) }
        }
    }

    @ViewBuilder
    private func cell(for entry: RemoteBrowserEntry) -> some View {
        if entry.isDirectory {
            NavigationLink {
                DirectoryView(root: root, path: entry.path, domain: domain)
            } label: {
                cellBody(entry)
            }
            .id("dir-\(entry.path)")
            .buttonStyle(.plain)
            .contextMenu { contextMenu(for: entry) }
        } else {
            Button {
                Task { await open(entry) }
            } label: {
                cellBody(entry)
            }
            .buttonStyle(.plain)
            .contextMenu { contextMenu(for: entry) }
        }
    }

    private func cellBody(_ entry: RemoteBrowserEntry) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Color.clear
                .aspectRatio(0.8, contentMode: .fit)
                .overlay {
                    ZStack(alignment: .bottomTrailing) {
                        cellThumbnail(entry)
                        if let duration = entry.durationSeconds, duration > 0 {
                            Text(pmlFormatTime(duration))
                                .font(.caption2.weight(.semibold))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 2)
                                .background(.black.opacity(0.55), in: Capsule())
                                .foregroundStyle(.white)
                                .padding(6)
                        }
                        if let progress = entry.progressSeconds, progress > 0,
                           let duration = entry.durationSeconds, duration > 0 {
                            GeometryReader { geo in
                                Rectangle()
                                    .fill(Color.accentColor)
                                    .frame(width: geo.size.width * CGFloat(min(max(Double(progress) / Double(duration), 0), 1)), height: 3)
                                    .frame(maxHeight: .infinity, alignment: .bottom)
                            }
                        }
                        if entry.isDirectory {
                            Image(systemName: "folder.fill")
                                .font(.caption2)
                                .padding(4)
                                .background(.ultraThinMaterial, in: Circle())
                                .padding(5)
                        }
                        if let assetId = entry.assetId, favorites.contains(assetId) {
                            Image(systemName: "heart.fill")
                                .font(.caption2)
                                .foregroundStyle(.pink)
                                .padding(4)
                                .background(.ultraThinMaterial, in: Circle())
                                .padding(5)
                                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                        }
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            Text(entry.name)
                .font(.caption2)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(metaLine(entry))
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)

        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .clipped()
    }

    @ViewBuilder
    private func cellThumbnail(_ entry: RemoteBrowserEntry) -> some View {
        let thumbId = entry.assetId ?? entry.coverAssetId
        ZStack {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(.quaternary.opacity(0.3))
            if let id = thumbId, let image = thumbs.images[id] {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Image(systemName: symbol(for: entry))
                    .font(.system(size: 24, weight: .semibold))
                    .foregroundStyle(.tint)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipped()
        .task(id: thumbId) {
            guard let id = thumbId else { return }
            await thumbs.image(assetId: id, client: model.apiClient)
        }
    }

    @ViewBuilder
    private func contextMenu(for entry: RemoteBrowserEntry) -> some View {
        if let assetId = entry.assetId {
            Button {
                Task { await toggleFavorite(assetId) }
            } label: {
                Label(
                    favorites.contains(assetId) ? "取消收藏" : "收藏",
                    systemImage: favorites.contains(assetId) ? "heart.slash" : "heart"
                )
            }
            Button {
                Task { await setAsDirectoryCover(assetId) }
            } label: {
                Label("设为当前目录封面", systemImage: "photo.badge.checkmark")
            }
        }
        Button {
            UIPasteboard.general.string = entry.path
        } label: {
            Label("复制路径", systemImage: "doc.on.doc")
        }
    }

    private func setAsDirectoryCover(_ assetId: String) async {
        guard let client = model.apiClient else { return }
        do {
            try await client.setDirectoryCover(rootId: root.id, remotePath: path, coverAssetId: assetId)
            noticeText = "已设为当前目录封面"
            await reload(force: true)
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func uploadDirectoryCover(_ item: PhotosPickerItem) async {
        guard let client = model.apiClient else { return }
        guard let data = try? await item.loadTransferable(type: Data.self),
              let base64 = pmlJPEGBase64(data, maxDimension: 1200) else {
            errorText = "读取图片失败"
            return
        }
        do {
            _ = try await client.uploadCustomCover(
                rootId: root.id,
                targetType: "directory",
                targetIdOrPath: path,
                dataBase64: base64
            )
            noticeText = "目录封面已更新"
            await reload(force: true)
        } catch {
            errorText = error.localizedDescription
        }
    }

    @ViewBuilder
    private func thumbnail(_ entry: RemoteBrowserEntry, size: CGFloat) -> some View {
        let thumbId = entry.assetId ?? entry.coverAssetId
        ZStack {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(.quaternary.opacity(0.3))
            if let id = thumbId, let image = thumbs.images[id] {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                Image(systemName: symbol(for: entry))
                    .font(.system(size: max(14, size * 0.34), weight: .semibold))
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .task(id: thumbId) {
            guard let id = thumbId else { return }
            await thumbs.image(assetId: id, client: model.apiClient)
        }
    }

    // MARK: - 数据

    private func bootstrap() async {
        layout = BrowsePreferences.shared.layout(rootId: root.id, path: path)
        sort = BrowsePreferences.shared.sort(rootId: root.id, path: path)
        await loadFavorites()
        await reload(force: false)
    }

    @MainActor
    private func loadFavorites() async {
        guard let client = model.apiClient else { return }
        if let ids = try? await client.favoriteIds() {
            favorites = Set(ids)
        }
    }

    @MainActor
    private func toggleFavorite(_ assetId: String) async {
        guard let client = model.apiClient else { return }
        if let nowFavorite = try? await client.toggleFavorite(assetId: assetId) {
            if nowFavorite {
                favorites.insert(assetId)
            } else {
                favorites.remove(assetId)
            }
        }
    }

    @MainActor
    private func reload(force: Bool, retried: Bool = false) async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            if let domain, force {
                // Do not hide provider failures behind a cached child listing.
                _ = try await client.refreshDomainPath(domain: domain, path: path)
            }
            // Use the compatibility DTO in both routes so directory covers,
            // thumbnails, duration and progress survive domain navigation too.
            let page = try await client.remoteChildren(
                rootId: root.id,
                path: path,
                refresh: force && domain == nil,
                sort: sort.rawValue
            )
            entries = page.items
            nextCursor = page.nextCursor
            if force {
                for id in page.items.compactMap({ $0.assetId ?? $0.coverAssetId }) {
                    thumbs.forget(assetId: id)
                }
            }
            prefetchThumbnails(page.items)
            if !force && entries.isEmpty && nextCursor == nil && !autoRefreshed {
                autoRefreshed = true
                let fresh = try await client.remoteChildren(
                    rootId: root.id,
                    path: path,
                    refresh: true,
                    sort: sort.rawValue
                )
                entries = fresh.items
                nextCursor = fresh.nextCursor
            }
            loading = false
        } catch {
            if pmlIsCancellation(error) {
                loading = false
                return
            }
            let message = error.localizedDescription
            // 会话在后台可能失效：自动重连后重试一次。
            if !retried && (message.contains("尚未连接") || message.contains("未连接")) {
                _ = try? await client.restoreConnection()
                loading = false
                await reload(force: force, retried: true)
                return
            }
            errorText = message
            loading = false
        }
    }

    @MainActor
    private func prefetchThumbnails(_ items: [RemoteBrowserEntry]) {
        let ids = items.prefix(24).compactMap { $0.assetId ?? $0.coverAssetId }
        guard !ids.isEmpty else { return }
        thumbs.prefetch(assetIds: Array(ids), client: model.apiClient)
    }

    private func loadMore(cursor: String) async {
        guard let client = model.apiClient, !loadingMore else { return }
        loadingMore = true
        do {
            let page = try await client.remoteChildren(
                rootId: root.id,
                path: path,
                cursor: cursor,
                sort: sort.rawValue
            )
            entries.append(contentsOf: page.items)
            nextCursor = page.nextCursor
            prefetchThumbnails(page.items)
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loadingMore = false
    }

    private func open(_ entry: RemoteBrowserEntry) async {
        guard let client = model.apiClient else { return }
        guard let assetId = entry.assetId else {
            errorText = "该文件没有可预览的索引资源"
            return
        }
        if entry.isComicArchive {
            comicTarget = ComicArchiveTarget(id: assetId, title: entry.name)
            return
        }
        if root.kind == "comics" && entry.isImage {
            folderComic = ComicFolderTarget(root: root, path: path, startPath: entry.path)
            return
        }
        if entry.isImage {
            if let index = imageEntries.firstIndex(where: { $0.id == entry.id }) {
                imageContext = ImageViewerContext(entries: imageEntries, startIndex: index)
            } else {
                imageContext = ImageViewerContext(entries: [entry], startIndex: 0)
            }
            return
        }
        do {
            guard let source = await model.playbackSource(assetId: assetId, remotePath: entry.path) else {
                errorText = "无法获取播放地址"
                return
            }
            let videos = visibleEntries.filter { $0.isVideo && $0.assetId != nil }
            videoPlaylist = videos.map {
                VideoPlaylistItem(id: $0.id, title: $0.name, assetId: $0.assetId, remotePath: $0.path)
            }
            videoPlaylistIndex = videos.firstIndex(where: { $0.id == entry.id })
            preview = PreviewItem(
                url: source.url,
                isVideo: entry.isVideo,
                assetId: assetId,
                title: entry.name,
                fallbackURL: source.fallbackURL,
                sourceLabel: source.label,
                remotePath: entry.path
            )
        } catch {
            errorText = error.localizedDescription
        }
    }

    // MARK: - 展示辅助

    private func symbol(for entry: RemoteBrowserEntry) -> String {
        if entry.isDirectory { return "folder.fill" }
        if entry.isVideo { return "play.rectangle.fill" }
        if entry.isComicArchive { return "book.closed.fill" }
        if entry.isImage { return "photo.fill" }
        return "doc.fill"
    }

    private func subtitle(_ entry: RemoteBrowserEntry) -> String {
        metaLine(entry)
    }

    private func metaLine(_ entry: RemoteBrowserEntry) -> String {
        let time = formattedModified(entry.modifiedAt)
        if entry.isDirectory {
            return time ?? "文件夹"
        }
        let size = pmlFormatBytes(entry.sizeBytes)
        if let time { return "\(size) · \(time)" }
        return size
    }

    private func formattedModified(_ raw: String?) -> String? {
        guard let date = modifiedDate(raw) else { return nil }
        return DirectoryView.modifiedDisplay.string(from: date)
    }

    private func modifiedDate(_ raw: String?) -> Date? {
        guard let raw, !raw.isEmpty else { return nil }
        if let date = DirectoryView.isoFractional.date(from: raw) ?? DirectoryView.isoPlain.date(from: raw) {
            return date
        }
        return DirectoryView.rfc2822.date(from: raw)
    }

    private static let modifiedDisplay: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "zh_CN")
        formatter.dateFormat = "yyyy-MM-dd HH:mm"
        return formatter
    }()

    private static let isoFractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let isoPlain: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    private static let rfc2822: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        return formatter
    }()
}
