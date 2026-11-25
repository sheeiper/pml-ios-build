import SwiftUI

private enum CollectionSortV2: String, CaseIterable, Identifiable {
    case count = "文件最多"
    case size = "体积最大"
    case name = "名称 A-Z"
    case recent = "最近更新"

    var id: String { rawValue }
    var apiValue: String {
        switch self {
        case .count: return "count"
        case .size: return "size"
        case .name: return "name"
        case .recent: return "recent"
        }
    }
}

// MARK: - 专题合集大厅 V2

/// 专题合集 V2：12 个频道横向筛选 + 4 宫格合集卡片 + 单视频外壳合集一键直放 + 纯文件夹层级浏览。
struct CollectionsView: View {
    @EnvironmentObject private var model: AppModel

    @State private var channels: [CollectionChannelStat] = []
    @State private var selectedChannel: String? = nil
    @State private var albums: [CollectionAlbumV2] = []
    @State private var totalAlbums: Int64 = 0
    @State private var searchText = ""
    @State private var sortMode: CollectionSortV2 = .count
    @State private var loading = false
    @State private var loadingMore = false
    @State private var syncing = false
    @State private var noticeText = ""
    @State private var errorText = ""
    @State private var playingVideo: V2VideoPlayPayload?

    private let pageSize = 60
    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                channelPillBar

                if !noticeText.isEmpty {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        Text(noticeText)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .pmlGlassCard(cornerRadius: 12)
                }

                if !errorText.isEmpty {
                    Text(errorText)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }

                if loading && albums.isEmpty {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("加载专题合集…").foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 44)
                } else if albums.isEmpty {
                    ContentUnavailableView(
                        "暂无合集",
                        systemImage: "square.stack.3d.up.slash",
                        description: Text("请切换频道或点击右上角同步合集模块。")
                    )
                    .padding(.top, 30)
                } else {
                    HStack(alignment: .firstTextBaseline) {
                        Text(selectedChannel ?? "全部合集")
                            .font(.headline)
                        Spacer()
                        Text("\(totalAlbums) 个合集")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }

                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(albums) { album in
                            if album.isSingleVideoWrapper, let singleId = album.singleVideoAssetId {
                                Button {
                                    Task { await playSingleVideo(assetId: singleId, title: album.name, remotePath: album.remotePath) }
                                } label: {
                                    V2FolderCardView(
                                        title: album.name,
                                        subtitle: album.channel,
                                        previewAssetIds: album.previewAssetIds,
                                        coverAssetId: album.coverAssetId,
                                        itemCount: album.itemCount,
                                        videoCount: album.videoCount,
                                        imageCount: album.imageCount,
                                        totalBytes: album.totalBytes,
                                        isSingleVideoWrapper: true,
                                        client: model.apiClient
                                    )
                                }
                                .buttonStyle(.plain)
                            } else {
                                NavigationLink {
                                    CollectionFolderView(
                                        albumId: album.id,
                                        albumTitle: album.name,
                                        subPath: nil
                                    )
                                } label: {
                                    V2FolderCardView(
                                        title: album.name,
                                        subtitle: album.channel,
                                        previewAssetIds: album.previewAssetIds,
                                        coverAssetId: album.coverAssetId,
                                        itemCount: album.itemCount,
                                        videoCount: album.videoCount,
                                        imageCount: album.imageCount,
                                        totalBytes: album.totalBytes,
                                        isSingleVideoWrapper: false,
                                        client: model.apiClient
                                    )
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }

                    if Int64(albums.count) < totalAlbums {
                        Button {
                            Task { await loadMore() }
                        } label: {
                            HStack(spacing: 8) {
                                if loadingMore { ProgressView() }
                                Text(loadingMore ? "加载中…" : "加载更多（已显示 \(albums.count)/\(totalAlbums)）")
                                    .font(.subheadline.weight(.medium))
                            }
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 12)
                            .pmlGlassCard(cornerRadius: 14)
                        }
                        .buttonStyle(.plain)
                        .disabled(loadingMore)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .navigationTitle("专题合集")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "搜索合集名称")
        .refreshable { await loadAll() }
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                Menu {
                    Picker("排序", selection: $sortMode) {
                        ForEach(CollectionSortV2.allCases) { mode in
                            Text(mode.rawValue).tag(mode)
                        }
                    }
                } label: {
                    Image(systemName: "arrow.up.arrow.down")
                }

                Button {
                    Task { await syncCollectionsModule() }
                } label: {
                    if syncing {
                        ProgressView()
                    } else {
                        Image(systemName: "arrow.clockwise")
                    }
                }
                .disabled(syncing)
                .accessibilityLabel("同步专题合集模块")
            }
        }
        .fullScreenCover(item: $playingVideo) { payload in
            GestureVideoPlayerSheet(
                url: payload.url,
                title: payload.title,
                assetId: payload.assetId,
                playlist: payload.playlist,
                playlistIndex: payload.playlistIndex,
                fallbackURL: payload.fallbackURL,
                sourceLabel: payload.sourceLabel,
                remotePath: payload.remotePath
            )
            .id(payload.id)
            .environmentObject(model)
        }
        .task { await loadAll() }
        .onChange(of: selectedChannel) { _, _ in
            Task { await loadAlbumsOnly() }
        }
        .onChange(of: sortMode) { _, _ in
            Task { await loadAlbumsOnly() }
        }
        .onChange(of: searchText) { _, _ in
            Task { await loadAlbumsOnly() }
        }
    }

    private var channelPillBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                V2FilterPill(
                    title: "全部频道",
                    count: channels.isEmpty ? nil : channels.reduce(0) { $0 + $1.albumCount },
                    isSelected: selectedChannel == nil
                ) {
                    selectedChannel = nil
                }

                ForEach(channels) { channel in
                    V2FilterPill(
                        title: channel.channel,
                        count: channel.albumCount,
                        isSelected: selectedChannel == channel.channel
                    ) {
                        selectedChannel = channel.channel
                    }
                }
            }
            .padding(.vertical, 2)
        }
    }

    private func loadAll() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            async let chReq = client.listCollectionChannels()
            async let albumReq = client.listCollectionAlbums(
                channel: selectedChannel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
                limit: pageSize,
                offset: 0
            )
            let (loadedChannels, page) = try await (chReq, albumReq)
            channels = loadedChannels
            albums = page.albums
            totalAlbums = page.total
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func loadAlbumsOnly() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            let page = try await client.listCollectionAlbums(
                channel: selectedChannel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
                limit: pageSize,
                offset: 0
            )
            albums = page.albums
            totalAlbums = page.total
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func loadMore() async {
        guard let client = model.apiClient, !loadingMore else { return }
        loadingMore = true
        defer { loadingMore = false }
        do {
            let page = try await client.listCollectionAlbums(
                channel: selectedChannel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
                limit: pageSize,
                offset: albums.count
            )
            albums.append(contentsOf: page.albums)
            totalAlbums = page.total
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func syncCollectionsModule() async {
        guard let client = model.apiClient, !syncing else { return }
        syncing = true
        noticeText = ""
        errorText = ""
        defer { syncing = false }
        do {
            let result = try await client.scanLibraryModule(moduleId: "collections", full: false)
            noticeText = "合集同步完成：扫描 \(result.scan.scanned) 项，共索引 \(result.rebuild.entityCount) 个合集"
            await loadAll()
        } catch {
            errorText = "同步失败：\(error.localizedDescription)"
        }
    }

    private func playSingleVideo(assetId: String, title: String, remotePath: String?) async {
        errorText = ""
        guard let source = await model.playbackSource(assetId: assetId, remotePath: remotePath) else {
            errorText = model.playbackError ?? "无法解析视频直链"
            return
        }
        playingVideo = V2VideoPlayPayload(
            url: source.url,
            title: title,
            assetId: assetId,
            playlist: [VideoPlaylistItem(id: assetId, title: title, assetId: assetId, remotePath: remotePath)],
            playlistIndex: 0,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: remotePath
        )
    }
}

// MARK: - 合集子目录层级视图（纯文件夹层级 + 单视频自动解包直放）

struct CollectionFolderView: View {
    let albumId: String
    let albumTitle: String
    let subPath: String?

    @EnvironmentObject private var model: AppModel
    @State private var album: CollectionAlbumV2?
    @State private var page: FolderBrowsePage?
    @State private var loading = false
    @State private var errorText = ""
    @State private var playingVideo: V2VideoPlayPayload?
    @State private var imageViewerContext: ImageViewerContext?
    @State private var comicTarget: ComicArchiveTarget?

    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
                if !errorText.isEmpty {
                    Text(errorText)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }

                if loading && page == nil {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("加载合集目录…").foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 44)
                } else if let page {
                    if !page.breadcrumbs.isEmpty {
                        Text(page.breadcrumbs.joined(separator: " / "))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    if !page.folders.isEmpty {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("文件夹（\(page.folders.count)）")
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(.secondary)
                            LazyVGrid(columns: columns, spacing: 12) {
                                ForEach(page.folders) { folder in
                                    if folder.isSingleVideoWrapper, let singleId = folder.singleVideoAssetId {
                                        Button {
                                            Task { await playSingleVideo(assetId: singleId, title: folder.name) }
                                        } label: {
                                            V2FolderCardView(
                                                title: folder.name,
                                                subtitle: nil,
                                                previewAssetIds: folder.previewAssetIds,
                                                coverAssetId: folder.coverAssetId,
                                                itemCount: folder.itemCount,
                                                videoCount: folder.videoCount,
                                                imageCount: folder.imageCount,
                                                totalBytes: folder.totalBytes,
                                                isSingleVideoWrapper: true,
                                                client: model.apiClient
                                            )
                                        }
                                        .buttonStyle(.plain)
                                    } else {
                                        NavigationLink {
                                            CollectionFolderView(
                                                albumId: albumId,
                                                albumTitle: folder.name,
                                                subPath: folder.subPath
                                            )
                                        } label: {
                                            V2FolderCardView(
                                                title: folder.name,
                                                subtitle: nil,
                                                previewAssetIds: folder.previewAssetIds,
                                                coverAssetId: folder.coverAssetId,
                                                itemCount: folder.itemCount,
                                                videoCount: folder.videoCount,
                                                imageCount: folder.imageCount,
                                                totalBytes: folder.totalBytes,
                                                isSingleVideoWrapper: false,
                                                client: model.apiClient
                                            )
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                            }
                        }
                    }

                    if !page.files.isEmpty {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("文件（\(page.files.count)）")
                                .font(.footnote.weight(.semibold))
                                .foregroundStyle(.secondary)
                            LazyVGrid(columns: columns, spacing: 12) {
                                ForEach(page.files) { asset in
                                    Button {
                                        Task { await openAsset(asset, siblings: page.files) }
                                    } label: {
                                        V2AssetCardView(asset: asset, client: model.apiClient)
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    }

                    if page.folders.isEmpty && page.files.isEmpty {
                        ContentUnavailableView(
                            "目录为空",
                            systemImage: "folder",
                            description: Text("当前目录下没有文件。")
                        )
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .navigationTitle(albumTitle)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .fullScreenCover(item: $playingVideo) { payload in
            GestureVideoPlayerSheet(
                url: payload.url,
                title: payload.title,
                assetId: payload.assetId,
                playlist: payload.playlist,
                playlistIndex: payload.playlistIndex,
                fallbackURL: payload.fallbackURL,
                sourceLabel: payload.sourceLabel,
                remotePath: payload.remotePath
            )
            .id(payload.id)
            .environmentObject(model)
        }
        .fullScreenCover(item: $imageViewerContext) { ctx in
            ImageViewer(context: ctx)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target)
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            let resp = try await client.browseCollectionFolder(
                albumId: albumId,
                subPath: subPath,
                limit: 300
            )
            album = resp.album
            page = resp.page
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func playSingleVideo(assetId: String, title: String) async {
        errorText = ""
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else {
            errorText = model.playbackError ?? "无法解析视频直链"
            return
        }
        playingVideo = V2VideoPlayPayload(
            url: source.url,
            title: title,
            assetId: assetId,
            playlist: [VideoPlaylistItem(id: assetId, title: title, assetId: assetId)],
            playlistIndex: 0,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: nil
        )
    }

    private func openAsset(_ asset: CreatorAssetV2, siblings: [CreatorAssetV2]) async {
        errorText = ""
        if asset.isVideo {
            let videos = siblings.filter(\.isVideo)
            let playlist = videos.map {
                VideoPlaylistItem(id: $0.id, title: $0.fileName, assetId: $0.id, remotePath: $0.remotePath)
            }
            let idx = videos.firstIndex(where: { $0.id == asset.id })
            guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else {
                errorText = model.playbackError ?? "无法解析视频直链"
                return
            }
            playingVideo = V2VideoPlayPayload(
                url: source.url,
                title: asset.fileName,
                assetId: asset.id,
                playlist: playlist,
                playlistIndex: idx,
                fallbackURL: source.fallbackURL,
                sourceLabel: source.label,
                remotePath: asset.remotePath
            )
        } else if asset.isComicArchive {
            comicTarget = ComicArchiveTarget(id: asset.id, title: asset.fileName)
        } else if asset.isImage {
            let images = siblings.filter(\.isImage)
            let entries = images.map { $0.toRemoteBrowserEntry() }
            let startIdx = images.firstIndex(where: { $0.id == asset.id }) ?? 0
            imageViewerContext = ImageViewerContext(entries: entries, startIndex: startIdx)
        }
    }
}
