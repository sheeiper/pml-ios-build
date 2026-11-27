import SwiftUI
import UIKit

/// 首页 V2：五大模块直达矩阵、新增文件（独立配置 Inbox）、影视库精选、常看创作者、专题合集、写真图集、漫画馆藏与扫描入口。
struct HomeView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase

    @State private var modules: [LibraryModuleInfo] = []
    @State private var inboxPath = ""
    @State private var inboxRoot: LibraryRoot?
    @State private var inboxEntries: [RemoteBrowserEntry] = []
    @State private var syncingInbox = false

    @State private var featuredMovies: [MovieMetadata] = []
    @State private var shufflingMovies = false
    @State private var creatorsV2: [CreatorSummaryV2] = []
    @State private var featuredCollections: [CollectionAlbumV2] = []
    @State private var featuredGalleries: [GallerySetV2] = []
    @State private var comics: [LibraryAsset] = []
    @State private var roots: [LibraryRoot] = []

    @State private var loading = true
    @State private var hasLoadedOnce = false
    @State private var checking = false
    @State private var noticeText = ""
    @State private var errorText = ""

    @State private var playingVideo: V2VideoPlayPayload?
    @State private var imageViewerContext: ImageViewerContext?
    @State private var archiveComic: ComicArchiveTarget?
    @State private var archiveGallery: ComicArchiveTarget?
    @State private var galleryReaderTarget: HomeGalleryReaderTarget?
    @State private var directoryTarget: HomeDirectoryTarget?
    @State private var selectedMovie: MovieMetadata?
    @State private var openingGalleryId: String?

    private let moduleColumns = [
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
    ]

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 22) {
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

                    modulesMatrixSection
                    inboxSection
                    featuredMoviesSection
                    creatorsSection
                    collectionsSection
                    galleriesSection
                    comicsSection
                    scanCard
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
            }
            .navigationTitle("首页")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
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
                    .accessibilityLabel("检查更新")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 14) {
                        NavigationLink {
                            PendingCurationView()
                        } label: {
                            Image(systemName: "tray.and.arrow.down")
                        }
                        .accessibilityLabel("待收纳整理")

                        NavigationLink {
                            FavoritesView()
                        } label: {
                            Image(systemName: "heart")
                        }

                        NavigationLink {
                            SettingsView()
                        } label: {
                            Image(systemName: "gearshape")
                        }
                    }
                }
            }
            .task { await refresh() }
            .onChange(of: scenePhase) { _, phase in
                guard phase == .active else { return }
                if errorText.lowercased().contains("cancel") || modules.isEmpty {
                    Task { await refresh() }
                }
            }
            .refreshable { await refreshAndCheckProvider() }
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
                ImageViewer(context: ctx).environmentObject(model)
            }
            .fullScreenCover(item: $archiveComic) { target in
                ComicReaderView(target: target).environmentObject(model)
            }
            .fullScreenCover(item: $archiveGallery) { target in
                ComicReaderView(target: target, kind: .gallery).environmentObject(model)
            }
            .fullScreenCover(item: $galleryReaderTarget) { target in
                ComicGalleryReaderView(kind: .gallery, title: target.title, sources: target.sources)
                    .environmentObject(model)
            }
            .sheet(item: $directoryTarget) { target in
                NavigationStack {
                    DirectoryView(root: target.root, path: target.path)
                        .environmentObject(model)
                }
            }
            .sheet(item: $selectedMovie) { movie in
                MovieDetailSheet(number: movie.number)
                    .environmentObject(model)
            }
        }
    }

    // MARK: - 顶部五大模块直达矩阵

    @ViewBuilder
    private var modulesMatrixSection: some View {
        if !modules.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("五大资料库")
                        .font(.headline)
                    Spacer()
                    NavigationLink("全部应用") {
                        AppCenterView()
                    }
                    .font(.caption)
                }

                LazyVGrid(columns: moduleColumns, spacing: 10) {
                    ForEach(modules) { mod in
                        NavigationLink {
                            moduleDestination(for: mod.kind)
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: mod.symbol)
                                    .font(.system(size: 18, weight: .semibold))
                                    .foregroundStyle(.white)
                                    .frame(width: 36, height: 36)
                                    .background(moduleTint(mod.kind), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                                VStack(alignment: .leading, spacing: 2) {
                                    Text(mod.name)
                                        .font(.subheadline.weight(.semibold))
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                    Text("\(mod.entityCount) \(mod.entityUnitLabel)")
                                        .font(.caption2.monospacedDigit())
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                    Text(PMLFormatters.bytes(mod.totalBytes))
                                        .font(.caption2.monospacedDigit())
                                        .foregroundStyle(.tertiary)
                                        .lineLimit(1)
                                }
                                Spacer(minLength: 0)
                            }
                            .padding(11)
                            .pmlGlassCard(cornerRadius: 16)
                        }
                        .buttonStyle(PressableTileStyle())
                    }

                    NavigationLink {
                        PendingCurationView()
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: "tray.and.arrow.down.fill")
                                .font(.system(size: 18, weight: .semibold))
                                .foregroundStyle(.white)
                                .frame(width: 36, height: 36)
                                .background(Color.orange.gradient, in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                            VStack(alignment: .leading, spacing: 2) {
                                Text("待收纳整理")
                                    .font(.subheadline.weight(.semibold))
                                    .foregroundStyle(.primary)
                                    .lineLimit(1)
                                Text("五大模块分拣")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                Text("刷卡 / 批量")
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                    .lineLimit(1)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(11)
                        .pmlGlassCard(cornerRadius: 16)
                    }
                    .buttonStyle(PressableTileStyle())
                }
            }
        } else if loading && !hasLoadedOnce {
            VStack(alignment: .leading, spacing: 10) {
                PMLSkeletonBlock(cornerRadius: 6).frame(width: 96, height: 18)
                LazyVGrid(columns: moduleColumns, spacing: 10) {
                    ForEach(0..<6, id: \.self) { _ in
                        HStack(spacing: 10) {
                            PMLSkeletonBlock(cornerRadius: 10).frame(width: 36, height: 36)
                            VStack(alignment: .leading, spacing: 5) {
                                PMLSkeletonBlock(cornerRadius: 4).frame(width: 68, height: 12)
                                PMLSkeletonBlock(cornerRadius: 4).frame(width: 48, height: 10)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(11)
                        .pmlGlassCard(cornerRadius: 16)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func moduleDestination(for kind: String) -> some View {
        switch kind {
        case "movies": MovieLibraryView()
        case "creators": AuthorsView()
        case "collections": CollectionsView()
        case "galleries": ComicLibraryView(initialTab: "图集")
        case "comics": ComicLibraryView()
        default: BrowseView()
        }
    }

    private func moduleTint(_ kind: String) -> AnyShapeStyle {
        switch kind {
        case "movies": return AnyShapeStyle(Color.indigo.gradient)
        case "creators": return AnyShapeStyle(Color.pink.gradient)
        case "collections": return AnyShapeStyle(Color.cyan.gradient)
        case "galleries": return AnyShapeStyle(Color.orange.gradient)
        case "comics": return AnyShapeStyle(Color.green.gradient)
        default: return AnyShapeStyle(Color.accentColor.gradient)
        }
    }

    // MARK: - 板块 1：新增文件（单独配置的新增文件夹 Inbox）

    private var inboxSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("新增文件")
                        .font(.headline)
                    if !inboxPath.isEmpty {
                        Text(inboxPath)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer()
                Button {
                    Task { await syncInbox() }
                } label: {
                    if syncingInbox {
                        ProgressView()
                    } else {
                        Label("同步", systemImage: "bolt.fill")
                            .font(.caption.weight(.medium))
                    }
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .disabled(syncingInbox || inboxRoot == nil)

                if let root = inboxRoot, !inboxPath.isEmpty {
                    Button("全部") {
                        directoryTarget = HomeDirectoryTarget(root: root, path: inboxPath)
                    }
                    .font(.caption)
                }
            }

            if loading && !hasLoadedOnce && inboxEntries.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 11) {
                        ForEach(0..<4, id: \.self) { _ in
                            VStack(alignment: .leading, spacing: 6) {
                                PMLSkeletonBlock(cornerRadius: 12).frame(width: 144, height: 94)
                                PMLSkeletonBlock(cornerRadius: 4).frame(width: 118, height: 12)
                                PMLSkeletonBlock(cornerRadius: 4).frame(width: 82, height: 10)
                            }
                            .padding(7)
                            .pmlGlassCard(cornerRadius: 15)
                        }
                    }
                }
            } else if inboxEntries.isEmpty {
                HStack(spacing: 10) {
                    Image(systemName: "tray")
                        .foregroundStyle(.secondary)
                    Text(inboxPath.isEmpty ? "尚未配置新增文件夹路径（请在媒体源中设置）" : "新增文件夹暂无新入库文件")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(14)
                .pmlGlassCard(cornerRadius: 16)
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 11) {
                        ForEach(inboxEntries) { entry in
                            Button {
                                Task { await openInboxEntry(entry) }
                            } label: {
                                inboxCard(entry)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    private func inboxCard(_ entry: RemoteBrowserEntry) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack(alignment: .topLeading) {
                if let coverId = entry.coverAssetId ?? entry.assetId {
                    CachedImageView(
                        url: model.apiClient?.v2CoverURL(assetId: coverId, isArchive: entry.isComicArchive),
                        contentMode: .fill,
                        minHeight: 94,
                        maxPixel: 420
                    )
                    .frame(width: 144, height: 94)
                    .clipped()
                } else {
                    ZStack {
                        Color.secondary.opacity(0.14)
                        Image(systemName: entry.symbol)
                            .font(.title2)
                            .foregroundStyle(.tint)
                    }
                    .frame(width: 144, height: 94)
                }

                Text(entry.isDirectory ? "文件夹" : (entry.isVideo ? "视频" : "文件"))
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.black.opacity(0.58), in: Capsule())
                    .padding(6)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            Text(entry.name)
                .font(.caption.weight(.medium))
                .foregroundStyle(.primary)
                .lineLimit(2)
                .frame(width: 144, alignment: .leading)
        }
        .padding(7)
        .pmlGlassCard(cornerRadius: 15)
    }

    // MARK: - 板块 2：影视库精选推荐

    @ViewBuilder
    private var featuredMoviesSection: some View {
        if loading && !hasLoadedOnce && featuredMovies.isEmpty {
            homeHorizontalSkeletonSection(titleWidth: 88, cardWidth: 168, imageHeight: 104, count: 3)
        } else if !featuredMovies.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("影视库精选")
                        .font(.headline)
                    Spacer()
                    Button {
                        Task { await shuffleMovies() }
                    } label: {
                        if shufflingMovies {
                            ProgressView()
                        } else {
                            Label("换一批", systemImage: "shuffle")
                                .font(.caption.weight(.medium))
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    .disabled(shufflingMovies)
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 11) {
                        ForEach(featuredMovies) { movie in
                            Button {
                                selectedMovie = movie
                            } label: {
                                VStack(alignment: .leading, spacing: 6) {
                                    ZStack(alignment: .topLeading) {
                                        CachedImageView(
                                            url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) },
                                            contentMode: .fill,
                                            minHeight: 104,
                                            maxPixel: 480
                                        )
                                        .frame(width: 168, height: 104)
                                        .clipped()

                                        Text(movie.number)
                                            .font(.caption2.weight(.bold))
                                            .foregroundStyle(.white)
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(Color.black.opacity(0.66), in: Capsule())
                                            .padding(6)
                                    }
                                    .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

                                    Text(movie.title ?? movie.number)
                                        .font(.caption.weight(.medium))
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                        .frame(width: 168, alignment: .leading)
                                }
                                .padding(7)
                                .pmlGlassCard(cornerRadius: 15)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    // MARK: - 板块 3：常看创作者

    @ViewBuilder
    private var creatorsSection: some View {
        if loading && !hasLoadedOnce && creatorsV2.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                PMLSkeletonBlock(cornerRadius: 6).frame(width: 88, height: 18)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 14) {
                        ForEach(0..<6, id: \.self) { _ in
                            VStack(spacing: 6) {
                                Circle()
                                    .fill(Color.secondary.opacity(0.16))
                                    .frame(width: 62, height: 62)
                                    .pmlShimmer()
                                PMLSkeletonBlock(cornerRadius: 4).frame(width: 52, height: 10)
                            }
                        }
                    }
                }
            }
        } else if !creatorsV2.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("常看创作者")
                        .font(.headline)
                    Spacer()
                    NavigationLink("全部") {
                        AuthorsView()
                    }
                    .font(.caption)
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 14) {
                        ForEach(creatorsV2.prefix(14)) { creator in
                            NavigationLink {
                                AuthorProfileView(creator: creator)
                                    .environmentObject(model)
                            } label: {
                                VStack(spacing: 6) {
                                    creatorAvatarView(creator)
                                        .frame(width: 62, height: 62)
                                        .clipShape(Circle())
                                        .overlay(Circle().strokeBorder(Color.primary.opacity(0.1), lineWidth: 1))
                                    Text(creator.name)
                                        .font(.caption)
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                        .frame(width: 68)
                                }
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func creatorAvatarView(_ creator: CreatorSummaryV2) -> some View {
        if let url = model.apiClient?.v2CoverURL(assetId: creator.avatarAssetId ?? creator.coverAssetId) {
            CachedImageView(url: url, contentMode: .fill, minHeight: 62, maxPixel: 200)
        } else {
            ZStack {
                Circle().fill(.quaternary.opacity(0.35))
                Text(String(creator.name.prefix(1)))
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(.tint)
            }
        }
    }

    // MARK: - 板块 4：专题合集精选

    @ViewBuilder
    private var collectionsSection: some View {
        if loading && !hasLoadedOnce && featuredCollections.isEmpty {
            homeHorizontalSkeletonSection(titleWidth: 74, cardWidth: 162, imageHeight: 98, count: 3)
        } else if !featuredCollections.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("专题合集")
                        .font(.headline)
                    Spacer()
                    NavigationLink("全部") {
                        CollectionsView()
                    }
                    .font(.caption)
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 11) {
                        ForEach(featuredCollections) { alb in
                            if alb.isSingleVideoWrapper, let singleId = alb.singleVideoAssetId {
                                Button {
                                    Task { await playVideo(assetId: singleId, title: alb.name, remotePath: alb.remotePath) }
                                } label: {
                                    collectionMiniCard(alb)
                                }
                                .buttonStyle(.plain)
                            } else {
                                NavigationLink {
                                    CollectionFolderView(albumId: alb.id, albumTitle: alb.name, subPath: nil)
                                } label: {
                                    collectionMiniCard(alb)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            }
        }
    }

    private func collectionMiniCard(_ alb: CollectionAlbumV2) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                CachedImageView(
                    url: model.apiClient?.v2CoverURL(assetId: alb.coverAssetId ?? alb.previewAssetIds.first),
                    contentMode: .fill,
                    minHeight: 98,
                    maxPixel: 420
                )
                .frame(width: 162, height: 98)
                .clipped()

                VStack {
                    HStack {
                        Text(alb.channel)
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Color.black.opacity(0.6), in: Capsule())
                        Spacer()
                    }
                    Spacer()
                    HStack {
                        Spacer()
                        Text(alb.isSingleVideoWrapper ? "▶ 单视频" : "\(alb.itemCount) 项")
                            .font(.caption2.monospacedDigit().weight(.semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(alb.isSingleVideoWrapper ? Color.accentColor.opacity(0.88) : Color.black.opacity(0.62), in: Capsule())
                    }
                }
                .padding(6)
            }
            .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

            Text(alb.name)
                .font(.caption.weight(.medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .frame(width: 162, alignment: .leading)
        }
        .padding(7)
        .pmlGlassCard(cornerRadius: 15)
    }

    // MARK: - 板块 5：写真图集精选

    @ViewBuilder
    private var galleriesSection: some View {
        if loading && !hasLoadedOnce && featuredGalleries.isEmpty {
            homeHorizontalSkeletonSection(titleWidth: 74, cardWidth: 122, imageHeight: 164, count: 4)
        } else if !featuredGalleries.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("写真图集")
                        .font(.headline)
                    Spacer()
                    NavigationLink("全部") {
                        ComicLibraryView(initialTab: "图集")
                    }
                    .font(.caption)
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 11) {
                        ForEach(featuredGalleries) { set in
                            Button {
                                Task { await openGallerySet(set) }
                            } label: {
                                VStack(alignment: .leading, spacing: 6) {
                                    ZStack {
                                        CachedImageView(
                                            url: model.apiClient?.v2CoverURL(assetId: set.coverAssetId, isArchive: set.isArchive, pageIndex: set.coverPageIndex ?? 0),
                                            contentMode: .fill,
                                            minHeight: 164,
                                            maxPixel: 420
                                        )
                                        .frame(width: 122, height: 164)
                                        .clipped()

                                        if openingGalleryId == set.id {
                                            Color.black.opacity(0.35)
                                            ProgressView().tint(.white)
                                        }

                                        VStack {
                                            Spacer()
                                            HStack {
                                                Spacer()
                                                Text(set.isArchive ? "ZIP" : "\(set.itemCount) P")
                                                    .font(.caption2.monospacedDigit().weight(.semibold))
                                                    .foregroundStyle(.white)
                                                    .padding(.horizontal, 6)
                                                    .padding(.vertical, 2)
                                                    .background(Color.black.opacity(0.65), in: Capsule())
                                            }
                                        }
                                        .padding(6)
                                    }
                                    .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

                                    Text(set.albumName)
                                        .font(.caption.weight(.medium))
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                        .frame(width: 122, alignment: .leading)
                                    Text(set.modelName)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                        .frame(width: 122, alignment: .leading)
                                }
                                .padding(7)
                                .pmlGlassCard(cornerRadius: 15)
                            }
                            .buttonStyle(.plain)
                            .disabled(openingGalleryId != nil)
                        }
                    }
                }
            }
        }
    }

    // MARK: - 板块 6：漫画馆藏 + 扫描中心

    @ViewBuilder
    private var comicsSection: some View {
        if loading && !hasLoadedOnce && comics.isEmpty {
            homeHorizontalSkeletonSection(titleWidth: 74, cardWidth: 112, imageHeight: 150, count: 4)
        } else if !comics.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text("漫画馆藏").font(.headline)
                    Spacer()
                    NavigationLink("全部") {
                        ComicLibraryView()
                    }
                    .font(.caption)
                }
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(comics) { asset in
                            Button {
                                Task { await openComic(asset) }
                            } label: {
                                VStack(alignment: .leading, spacing: 6) {
                                    CachedImageView(
                                        url: model.apiClient?.v2CoverURL(assetId: asset.thumbnailAssetId ?? asset.id, isArchive: isComicArchiveName(asset.fileName)),
                                        contentMode: .fill,
                                        minHeight: 150,
                                        maxPixel: 420
                                    )
                                    .frame(width: 112, height: 150)
                                    .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

                                    Text(asset.fileName)
                                        .font(.caption)
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                        .frame(width: 112, alignment: .leading)
                                }
                                .padding(7)
                                .pmlGlassCard(cornerRadius: 14)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    private var scanCard: some View {
        NavigationLink {
            ScanCenterView()
        } label: {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Label("五大模块同步与扫描中心", systemImage: "arrow.triangle.2.circlepath")
                        .font(.headline)
                    Spacer()
                    Text("进入").font(.caption).foregroundStyle(.tint)
                }
                Text("支持影视库、创作者、合集、图集、漫画独立增量同步与索引重建。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(15)
            .pmlGlassCard(cornerRadius: 18)
        }
        .buttonStyle(.plain)
    }

    // MARK: - 数据加载与交互

    private func refreshAndCheckProvider() async {
        guard let client = model.apiClient, !checking else { return }
        checking = true
        noticeText = ""
        defer { checking = false }
        do {
            let outcomes = try await client.checkDomainRootsForUpdates()
            noticeText = outcomes.refreshNotice
            await refresh()
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
    }

    private func refresh() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer {
            loading = false
            hasLoadedOnce = true
        }

        async let modulesReq = client.listLibraryModules()
        async let configReq = client.directoryConfig()
        async let rootsReq = client.libraryRoots()
        async let moviesReq = client.getRandomFeaturedMovies(limit: 12)
        async let creatorsReq = client.listCreatorsV2(sort: "count")
        async let collectionsReq = client.listCollectionAlbums(sort: "recent", limit: 12, offset: 0)
        async let galleriesReq = client.listGallerySets(sort: "recent", limit: 12, offset: 0)
        async let comicsReq = client.libraryAssets(filter: "comics", limit: 16)

        modules = (try? await modulesReq) ?? []
        roots = (try? await rootsReq) ?? []
        featuredMovies = (try? await moviesReq) ?? []
        await EntityFavoritesStore.shared.ensureLoaded(client: client)
        let loadedCreators = (try? await creatorsReq) ?? []
        creatorsV2 = loadedCreators.sorted {
            let aFav = EntityFavoritesStore.shared.isFavorited(.creator, id: $0.id)
            let bFav = EntityFavoritesStore.shared.isFavorited(.creator, id: $1.id)
            if aFav != bFav { return aFav }
            return false
        }
        featuredCollections = (try? await collectionsReq)?.albums ?? []
        featuredGalleries = (try? await galleriesReq)?.sets ?? []
        comics = (try? await comicsReq) ?? []

        if let config = try? await configReq {
            let configuredInbox = config.recentPath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            inboxPath = configuredInbox
            inboxRoot = roots.first { configuredInbox == $0.remotePath || configuredInbox.hasPrefix($0.remotePath + "/") }
            if let root = inboxRoot, !configuredInbox.isEmpty {
                await loadInboxEntries(root: root, path: configuredInbox, forceRefresh: false)
            } else {
                inboxEntries = []
            }
        }
    }

    private func syncInbox() async {
        guard let root = inboxRoot, !inboxPath.isEmpty, !syncingInbox else { return }
        syncingInbox = true
        noticeText = ""
        defer { syncingInbox = false }
        await loadInboxEntries(root: root, path: inboxPath, forceRefresh: true)
        noticeText = "新增文件目录已同步（\(inboxEntries.count) 项）"
    }

    private func loadInboxEntries(root: LibraryRoot, path: String, forceRefresh: Bool) async {
        guard let client = model.apiClient else { return }
        if let page = try? await client.remoteChildren(rootId: root.id, path: path, refresh: forceRefresh, sort: "newest") {
            inboxEntries = page.items
                .filter { !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@") }
                .prefix(24)
                .map { $0 }
        }
    }

    private func shuffleMovies() async {
        guard let client = model.apiClient, !shufflingMovies else { return }
        shufflingMovies = true
        defer { shufflingMovies = false }
        if let fresh = try? await client.getRandomFeaturedMovies(limit: 12) {
            featuredMovies = fresh
        }
    }

    private func openInboxEntry(_ entry: RemoteBrowserEntry) async {
        if entry.isDirectory, let root = inboxRoot {
            directoryTarget = HomeDirectoryTarget(root: root, path: entry.path)
            return
        }
        if entry.isVideo {
            await playVideo(assetId: entry.assetId ?? "", title: entry.name, remotePath: entry.path)
        } else if entry.isComicArchive, let assetId = entry.assetId {
            archiveComic = ComicArchiveTarget(id: assetId, title: entry.name)
        } else if entry.isImage {
            let images = inboxEntries.filter(\.isImage)
            let idx = images.firstIndex(where: { $0.id == entry.id }) ?? 0
            imageViewerContext = ImageViewerContext(entries: images, startIndex: idx)
        }
    }

    private func playVideo(assetId: String, title: String, remotePath: String?) async {
        errorText = ""
        let resolvedId = assetId.isEmpty ? nil : assetId
        guard let source = await model.playbackSource(assetId: resolvedId, remotePath: remotePath) else {
            errorText = model.playbackError ?? "无法解析视频直链"
            return
        }
        playingVideo = V2VideoPlayPayload(
            url: source.url,
            title: title,
            assetId: assetId,
            playlist: [VideoPlaylistItem(id: assetId.isEmpty ? (remotePath ?? title) : assetId, title: title, assetId: resolvedId, remotePath: remotePath)],
            playlistIndex: 0,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: remotePath
        )
    }

    private func openGallerySet(_ item: GallerySetV2) async {
        guard let client = model.apiClient else { return }
        if item.isArchive, let coverId = item.coverAssetId, !coverId.isEmpty {
            archiveGallery = ComicArchiveTarget(id: coverId, title: item.albumName)
            return
        }
        openingGalleryId = item.id
        errorText = ""
        defer { openingGalleryId = nil }
        do {
            let assets = try await client.getGallerySetItems(setId: item.id)
            if item.isArchive, let archive = assets.first {
                archiveGallery = ComicArchiveTarget(id: archive.id, title: item.albumName)
                return
            }
            let images = assets.filter(\.isImage)
            if !images.isEmpty {
                let sources: [ReaderPageSource] = images.compactMap { img in
                    let fallback = model.alistDirectURL(remotePath: img.remotePath) ?? client.mediaSourceURL(assetId: img.id)
                    guard let fallback else { return nil }
                    return .remoteAsset(assetId: img.id, remotePath: img.remotePath, fallbackURL: fallback)
                }
                if !sources.isEmpty {
                    galleryReaderTarget = HomeGalleryReaderTarget(title: item.albumName, sources: sources)
                }
            } else if let archive = assets.first(where: \.isComicArchive) {
                archiveGallery = ComicArchiveTarget(id: archive.id, title: item.albumName)
            }
        } catch {
            errorText = "打开图集失败：\(error.localizedDescription)"
        }
    }

    private func homeHorizontalSkeletonSection(titleWidth: CGFloat, cardWidth: CGFloat, imageHeight: CGFloat, count: Int) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            PMLSkeletonBlock(cornerRadius: 6).frame(width: titleWidth, height: 18)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 11) {
                    ForEach(0..<count, id: \.self) { _ in
                        VStack(alignment: .leading, spacing: 6) {
                            PMLSkeletonBlock(cornerRadius: 11).frame(width: cardWidth, height: imageHeight)
                            PMLSkeletonBlock(cornerRadius: 4).frame(width: cardWidth * 0.78, height: 11)
                        }
                        .padding(7)
                        .pmlGlassCard(cornerRadius: 15)
                    }
                }
            }
        }
    }

    private func openComic(_ asset: LibraryAsset) async {
        if isComicArchiveName(asset.fileName) {
            archiveComic = ComicArchiveTarget(id: asset.id, title: asset.fileName)
            return
        }
        guard let root = roots.first(where: { $0.id == asset.rootId }) else { return }
        directoryTarget = HomeDirectoryTarget(
            root: root,
            path: asset.parentPath.isEmpty ? root.remotePath : asset.parentPath
        )
    }

    private func isComicArchiveName(_ name: String) -> Bool {
        let lower = name.lowercased()
        return lower.hasSuffix(".cbz") || lower.hasSuffix(".zip") || lower.hasSuffix(".cbr") || lower.hasSuffix(".rar") || lower.hasSuffix(".pdf")
    }
}

struct HomeDirectoryTarget: Identifiable {
    let id = UUID()
    let root: LibraryRoot
    let path: String
}

struct HomeGalleryReaderTarget: Identifiable {
    let id = UUID()
    let title: String
    let sources: [ReaderPageSource]
}
