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

/// 目录与收藏内存缓存：
/// 1. 秒开已访问或已预取的子目录（0ms 渲染，告别每次进入文件夹转圈等待）；
/// 2. 缓存全局收藏 ID 集合，消除每次打开文件夹前串行等待 `list_favorite_ids` 的网络往返；
/// 3. 打开父目录后后台静默预热前几个子文件夹，点击子文件夹直接命中内存。
@MainActor
final class DirectoryMemoryCache {
    static let shared = DirectoryMemoryCache()

    struct Snapshot {
        var entries: [RemoteBrowserEntry]
        var nextCursor: String?
        var updatedAt: Date
    }

    private var pages: [String: Snapshot] = [:]
    private var order: [String] = []
    private var prefetchTask: Task<Void, Never>?

    private(set) var favoriteIds: Set<String> = []
    private var favoritesUpdatedAt: Date?

    private static let maxCachedFolders = 160

    private func makeKey(rootId: String, path: String) -> String {
        let clean = path.trimmingCharacters(in: .whitespacesAndNewlines)
        return "\(rootId)|\(clean)"
    }

    func snapshot(rootId: String, path: String) -> Snapshot? {
        pages[makeKey(rootId: rootId, path: path)]
    }

    func store(entries: [RemoteBrowserEntry], nextCursor: String?, rootId: String, path: String) {
        let key = makeKey(rootId: rootId, path: path)
        if pages[key] == nil {
            order.append(key)
            if order.count > Self.maxCachedFolders {
                let overflow = order.count - Self.maxCachedFolders
                for old in order.prefix(overflow) {
                    pages.removeValue(forKey: old)
                }
                order.removeFirst(overflow)
            }
        }
        pages[key] = Snapshot(entries: entries, nextCursor: nextCursor, updatedAt: Date())
    }

    func invalidate(rootId: String, path: String) {
        pages.removeValue(forKey: makeKey(rootId: rootId, path: path))
    }

    func updateFavorites(_ ids: Set<String>) {
        favoriteIds = ids
        favoritesUpdatedAt = Date()
    }

    var shouldRefreshFavorites: Bool {
        guard let favoritesUpdatedAt else { return true }
        return Date().timeIntervalSince(favoritesUpdatedAt) > 60
    }

    func scheduleSubfolderPrefetch(
        subfolders: [RemoteBrowserEntry],
        root: LibraryRoot,
        domain: String?,
        sort: String,
        client: PMLAPIClient?
    ) {
        prefetchTask?.cancel()
        prefetchTask = nil
        guard let client, !subfolders.isEmpty else { return }
        let targets = subfolders.filter { snapshot(rootId: root.id, path: $0.path) == nil }.prefix(6)
        guard !targets.isEmpty else { return }

        prefetchTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 120_000_000)
            for folder in targets {
                if Task.isCancelled { break }
                if snapshot(rootId: root.id, path: folder.path) != nil { continue }
                do {
                    if let domain, root.id.hasPrefix("domain-") {
                        let children = try await client.domainChildren(domain: domain, path: folder.path)
                        if Task.isCancelled { break }
                        let mapped = children.map { child in
                            RemoteBrowserEntry(
                                path: child.path,
                                name: child.name,
                                isDirectory: child.isDirectory,
                                sizeBytes: child.sizeBytes,
                                modifiedAt: child.modifiedAt,
                                etag: nil,
                                assetId: child.assetId,
                                mediaType: child.mediaType,
                                thumbnailSource: nil,
                                coverAssetId: child.thumbnailAssetId
                            )
                        }
                        store(entries: mapped, nextCursor: nil, rootId: root.id, path: folder.path)
                    } else {
                        let page = try await client.remoteChildren(
                            rootId: root.id,
                            path: folder.path,
                            refresh: false,
                            sort: sort
                        )
                        if Task.isCancelled { break }
                        store(entries: page.items, nextCursor: page.nextCursor, rootId: root.id, path: folder.path)
                    }
                } catch {
                    continue
                }
            }
        }
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
    @State private var openTask: Task<Void, Never>?
    @State private var openingEntryPath: String?

    // 预计算与缓存的衍生状态，彻底消除每次视图刷新重复执行 O(N log N) 排序与 DateFormatter 解析
    @State private var cachedSortedEntries: [RemoteBrowserEntry] = []
    @State private var visibleEntries: [RemoteBrowserEntry] = []
    @State private var parsedTimestampByPath: [String: TimeInterval] = [:]
    @State private var formattedMetaByPath: [String: String] = [:]
    @State private var cachedPlayableVideos: [RemoteBrowserEntry] = []
    @State private var cachedResumeVideo: RemoteBrowserEntry?
    @State private var cachedFolderCoverId: String?
    @State private var cachedFolderMeta: String = "0 个文件"

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
        .onChange(of: searchText) { _, _ in
            applySearchFilter()
        }
        .refreshable { await reload(force: true) }
        .task(id: "\(root.id)|\(path)") { await bootstrap() }
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
                        recomputeEntries(resetDates: false)
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
            .id(item.id)
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
                        if let entry = resumeVideo ?? playableVideos.first {
                            triggerOpen(entry)
                        }
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
            AssetThumbnailView(assetId: cachedFolderCoverId, contentMode: .fill) {
                Image(systemName: root.kind == "comics" ? "book.closed.fill" : "folder.fill")
                    .font(.title2)
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: 92, height: 128)
        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
    }

    private var playableVideos: [RemoteBrowserEntry] {
        cachedPlayableVideos
    }

    private var resumeVideo: RemoteBrowserEntry? {
        cachedResumeVideo
    }

    private var folderCoverId: String? {
        cachedFolderCoverId
    }

    private var folderMeta: String {
        cachedFolderMeta
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
        .clipped()
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
                triggerOpen(entry)
            } label: {
                content
            }
            .id("file-\(entry.path)")
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
                triggerOpen(entry)
            } label: {
                cellBody(entry)
            }
            .id("file-\(entry.path)")
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
                    .allowsHitTesting(false)
                }
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .clipped()

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
        .clipped()
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private func cellThumbnail(_ entry: RemoteBrowserEntry) -> some View {
        let thumbId = entry.assetId ?? entry.coverAssetId
        ZStack {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(.quaternary.opacity(0.3))
            AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                Image(systemName: symbol(for: entry))
                    .font(.system(size: 24, weight: .semibold))
                    .foregroundStyle(.tint)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipped()
        .allowsHitTesting(false)
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
            AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                Image(systemName: symbol(for: entry))
                    .font(.system(size: max(14, size * 0.34), weight: .semibold))
                    .foregroundStyle(.tint)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .clipped()
        .allowsHitTesting(false)
    }

    // MARK: - 数据与预计算缓存

    @MainActor
    private func recomputeEntries(resetDates: Bool) {
        var timestamps = resetDates ? [:] : parsedTimestampByPath
        var metas = resetDates ? [:] : formattedMetaByPath

        for entry in entries {
            if timestamps[entry.path] == nil {
                if let date = modifiedDate(entry.modifiedAt) {
                    timestamps[entry.path] = date.timeIntervalSince1970
                    let timeStr = DirectoryView.modifiedDisplay.string(from: date)
                    if entry.isDirectory {
                        metas[entry.path] = timeStr
                    } else {
                        metas[entry.path] = "\(pmlFormatBytes(entry.sizeBytes)) · \(timeStr)"
                    }
                } else {
                    timestamps[entry.path] = -Double.greatestFiniteMagnitude
                    metas[entry.path] = entry.isDirectory ? "文件夹" : pmlFormatBytes(entry.sizeBytes)
                }
            }
        }
        parsedTimestampByPath = timestamps
        formattedMetaByPath = metas

        let currentSort = sort
        cachedSortedEntries = entries.sorted { left, right in
            if left.isDirectory != right.isDirectory { return left.isDirectory }
            switch currentSort {
            case .name:
                break
            case .newest:
                let l = timestamps[left.path] ?? -Double.greatestFiniteMagnitude
                let r = timestamps[right.path] ?? -Double.greatestFiniteMagnitude
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
        applySearchFilter()
    }

    @MainActor
    private func applySearchFilter() {
        let trimmed = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let currentVisible: [RemoteBrowserEntry]
        if trimmed.isEmpty {
            currentVisible = cachedSortedEntries
        } else {
            currentVisible = cachedSortedEntries.filter { $0.name.localizedCaseInsensitiveContains(trimmed) }
        }
        visibleEntries = currentVisible

        let videos = currentVisible.filter { $0.isVideo && $0.assetId != nil }
        cachedPlayableVideos = videos
        cachedResumeVideo = videos
            .filter { ($0.progressSeconds ?? 0) > 2 }
            .max { ($0.progressSeconds ?? 0) < ($1.progressSeconds ?? 0) }
        cachedFolderCoverId = currentVisible.compactMap(\.coverAssetId).first
            ?? currentVisible.first(where: { !$0.isDirectory })?.assetId

        var folderCount = 0
        var fileCount = 0
        var maxTimestamp: TimeInterval = -Double.greatestFiniteMagnitude
        for entry in currentVisible {
            if entry.isDirectory {
                folderCount += 1
            } else {
                fileCount += 1
            }
            if let ts = parsedTimestampByPath[entry.path], ts > maxTimestamp {
                maxTimestamp = ts
            }
        }
        var parts = ["\(fileCount) 个文件"]
        if folderCount > 0 { parts.append("\(folderCount) 个文件夹") }
        if maxTimestamp > -Double.greatestFiniteMagnitude / 2 {
            let latestDate = Date(timeIntervalSince1970: maxTimestamp)
            parts.append(DirectoryView.modifiedDisplay.string(from: latestDate))
        }
        cachedFolderMeta = parts.joined(separator: " · ")
    }

    @MainActor
    private func bootstrap() async {
        layout = BrowsePreferences.shared.layout(rootId: root.id, path: path)
        sort = BrowsePreferences.shared.sort(rootId: root.id, path: path)

        // 1. 立即同步读取内存中的收藏集与目录快照（0ms 响应，彻底砍掉串行等待 list_favorite_ids 的耗时）
        favorites = DirectoryMemoryCache.shared.favoriteIds
        if DirectoryMemoryCache.shared.shouldRefreshFavorites {
            Task { @MainActor in
                await loadFavorites()
            }
        }

        if let cached = DirectoryMemoryCache.shared.snapshot(rootId: root.id, path: path) {
            entries = cached.entries
            nextCursor = cached.nextCursor
            recomputeEntries(resetDates: true)
            loading = false
            prefetchThumbnails(visibleEntries)
            scheduleSubfolderPrefetch()
            // 缓存未超过 45 秒直接复用，超过则后台静默刷新
            if Date().timeIntervalSince(cached.updatedAt) > 45 {
                await reload(force: false, silent: true)
            }
            return
        }

        await reload(force: false, silent: false)
    }

    @MainActor
    private func loadFavorites() async {
        guard let client = model.apiClient else { return }
        if let ids = try? await client.favoriteIds() {
            let set = Set(ids)
            favorites = set
            DirectoryMemoryCache.shared.updateFavorites(set)
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
            DirectoryMemoryCache.shared.updateFavorites(favorites)
        }
    }

    private func fetchPage(
        client: PMLAPIClient,
        refresh: Bool,
        cursor: String? = nil
    ) async throws -> RemoteBrowserPage {
        if let domain, root.id.hasPrefix("domain-") {
            if refresh {
                _ = try? await client.refreshDomainPath(domain: domain, path: path)
            }
            let children = try await client.domainChildren(domain: domain, path: path)
            let items = children.map { child in
                RemoteBrowserEntry(
                    path: child.path,
                    name: child.name,
                    isDirectory: child.isDirectory,
                    sizeBytes: child.sizeBytes,
                    modifiedAt: child.modifiedAt,
                    etag: nil,
                    assetId: child.assetId,
                    mediaType: child.mediaType,
                    thumbnailSource: nil,
                    coverAssetId: child.thumbnailAssetId
                )
            }
            return RemoteBrowserPage(items: items, nextCursor: nil, fromLocalCache: !refresh)
        }
        return try await client.remoteChildren(
            rootId: root.id,
            path: path,
            cursor: cursor,
            refresh: refresh,
            sort: sort.rawValue
        )
    }

    @MainActor
    private func reload(force: Bool, retried: Bool = false, silent: Bool = false) async {
        guard let client = model.apiClient else { return }
        if !silent {
            loading = true
        }
        errorText = ""
        do {
            if let domain, force, !root.id.hasPrefix("domain-") {
                _ = try await client.refreshDomainPath(domain: domain, path: path)
            }
            let page = try await fetchPage(client: client, refresh: force && domain == nil)
            entries = page.items
            nextCursor = page.nextCursor
            DirectoryMemoryCache.shared.store(
                entries: page.items,
                nextCursor: page.nextCursor,
                rootId: root.id,
                path: path
            )
            if force {
                for id in page.items.compactMap({ $0.assetId ?? $0.coverAssetId }) {
                    ThumbnailService.shared.forget(assetId: id)
                }
            }
            recomputeEntries(resetDates: true)
            prefetchThumbnails(visibleEntries)
            scheduleSubfolderPrefetch()

            if !force && entries.isEmpty && nextCursor == nil && !autoRefreshed {
                autoRefreshed = true
                let fresh = try await fetchPage(client: client, refresh: true)
                entries = fresh.items
                nextCursor = fresh.nextCursor
                DirectoryMemoryCache.shared.store(
                    entries: fresh.items,
                    nextCursor: fresh.nextCursor,
                    rootId: root.id,
                    path: path
                )
                recomputeEntries(resetDates: true)
                prefetchThumbnails(visibleEntries)
                scheduleSubfolderPrefetch()
            }
            loading = false
        } catch {
            if pmlIsCancellation(error) {
                loading = false
                return
            }
            let message = error.localizedDescription
            if !retried && (message.contains("尚未连接") || message.contains("未连接")) {
                _ = try? await client.restoreConnection()
                loading = false
                await reload(force: force, retried: true, silent: silent)
                return
            }
            if !silent || entries.isEmpty {
                errorText = message
            }
            loading = false
        }
    }

    @MainActor
    private func prefetchThumbnails(_ items: [RemoteBrowserEntry]) {
        let ids = items.prefix(8).compactMap { $0.assetId ?? $0.coverAssetId }
        guard !ids.isEmpty else { return }
        ThumbnailService.shared.prefetch(assetIds: Array(ids), client: model.apiClient)
    }

    @MainActor
    private func scheduleSubfolderPrefetch() {
        let subfolders = visibleEntries.filter(\.isDirectory)
        guard !subfolders.isEmpty else { return }
        DirectoryMemoryCache.shared.scheduleSubfolderPrefetch(
            subfolders: subfolders,
            root: root,
            domain: domain,
            sort: sort.rawValue,
            client: model.apiClient
        )
    }

    private func loadMore(cursor: String) async {
        guard let client = model.apiClient, !loadingMore else { return }
        loadingMore = true
        do {
            let page = try await fetchPage(client: client, refresh: false, cursor: cursor)
            entries.append(contentsOf: page.items)
            nextCursor = page.nextCursor
            DirectoryMemoryCache.shared.store(
                entries: entries,
                nextCursor: nextCursor,
                rootId: root.id,
                path: path
            )
            recomputeEntries(resetDates: false)
            prefetchThumbnails(page.items)
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loadingMore = false
    }

    @MainActor
    private func triggerOpen(_ entry: RemoteBrowserEntry) {
        openTask?.cancel()
        openingEntryPath = entry.path
        openTask = Task { @MainActor in
            await open(entry)
        }
    }

    @MainActor
    private func open(_ entry: RemoteBrowserEntry) async {
        guard model.apiClient != nil else { return }
        let requestedPath = entry.path
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
            if let index = imageEntries.firstIndex(where: { $0.path == requestedPath }) {
                imageContext = ImageViewerContext(entries: imageEntries, startIndex: index)
            } else {
                imageContext = ImageViewerContext(entries: [entry], startIndex: 0)
            }
            return
        }
        guard let source = await model.playbackSource(assetId: assetId, remotePath: requestedPath) else {
            if openingEntryPath == requestedPath {
                errorText = "无法获取播放地址"
            }
            return
        }
        guard !Task.isCancelled, openingEntryPath == requestedPath else { return }
        let videos = cachedPlayableVideos
        videoPlaylist = videos.map {
            VideoPlaylistItem(id: $0.path, title: $0.name, assetId: $0.assetId, remotePath: $0.path)
        }
        videoPlaylistIndex = videos.firstIndex(where: { $0.path == requestedPath })
        preview = PreviewItem(
            url: source.url,
            isVideo: entry.isVideo,
            assetId: assetId,
            title: entry.name,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: requestedPath
        )
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
        if let cached = formattedMetaByPath[entry.path] {
            return cached
        }
        if entry.isDirectory {
            return "文件夹"
        }
        return pmlFormatBytes(entry.sizeBytes)
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
