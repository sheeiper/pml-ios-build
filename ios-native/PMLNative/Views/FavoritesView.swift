import SwiftUI

private enum FavoritesSectionTab: String, CaseIterable, Identifiable {
    case movies = "影片"
    case performers = "演员"
    case creators = "创作者"
    case files = "单文件"

    var id: String { rawValue }
}

/// 统一收藏中心：支持 影片 / 演员 / 创作者 / 单文件 四个分区。
struct FavoritesView: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var favs = EntityFavoritesStore.shared

    @State private var activeTab: FavoritesSectionTab = .movies
    @State private var movies: [MovieMetadata] = []
    @State private var performers: [MoviePerformer] = []
    @State private var creators: [CreatorSummaryV2] = []
    @State private var assets: [LibraryAsset] = []

    @State private var loading = false
    @State private var errorText = ""
    @State private var selectedMovie: MovieMetadata?
    @State private var selectedPerformer: MoviePerformer?
    @State private var preview: PreviewItem?
    @State private var imageContext: ImageViewerContext?

    private let twoColumns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]
    private let threeColumns = [GridItem(.adaptive(minimum: 104), spacing: 10)]

    var body: some View {
        VStack(spacing: 0) {
            Picker("收藏分类", selection: $activeTab) {
                ForEach(FavoritesSectionTab.allCases) { tab in
                    Text(tab.rawValue).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)

            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if !errorText.isEmpty {
                        Text(errorText)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }

                    if loading && currentIsEmpty {
                        ProgressView("加载收藏…")
                            .frame(maxWidth: .infinity)
                            .padding(.top, 40)
                    } else {
                        switch activeTab {
                        case .movies:
                            moviesGrid
                        case .performers:
                            performersGrid
                        case .creators:
                            creatorsGrid
                        case .files:
                            filesGrid
                        }
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
            }
            .refreshable { await loadCurrentTab() }
        }
        .navigationTitle("收藏")
        .navigationBarTitleDisplayMode(.inline)
        .task { await loadCurrentTab() }
        .onChange(of: activeTab) { _, _ in
            Task { await loadCurrentTab() }
        }
        .sheet(item: $selectedMovie) { movie in
            MovieDetailSheet(
                number: movie.number,
                assetId: movie.assetId,
                fileName: movie.title,
                remotePath: nil,
                onPlay: { targetId in
                    selectedMovie = nil
                    Task {
                        try? await Task.sleep(nanoseconds: 420_000_000)
                        await playAsset(assetId: targetId, title: movie.title)
                    }
                }
            )
            .environmentObject(model)
        }
        .sheet(item: $selectedPerformer) { person in
            MoviePerformerDetailSheet(
                performer: person,
                onPlay: { assetId, title in
                    selectedPerformer = nil
                    Task {
                        try? await Task.sleep(nanoseconds: 420_000_000)
                        await playAsset(assetId: assetId, title: title)
                    }
                }
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $preview) { item in
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
        }
        .fullScreenCover(item: $imageContext) { context in
            ImageViewer(context: context)
                .environmentObject(model)
        }
    }

    private var currentIsEmpty: Bool {
        switch activeTab {
        case .movies: return movies.isEmpty
        case .performers: return performers.isEmpty
        case .creators: return creators.isEmpty
        case .files: return assets.isEmpty
        }
    }

    // MARK: - 1. 收藏的影片

    @ViewBuilder
    private var moviesGrid: some View {
        if movies.isEmpty {
            ContentUnavailableView("暂无收藏影片", systemImage: "film", description: Text("在影视库或影片详情点击爱心即可收藏。"))
                .padding(.top, 30)
        } else {
            LazyVGrid(columns: twoColumns, spacing: 12) {
                ForEach(movies) { movie in
                    Button {
                        selectedMovie = movie
                    } label: {
                        VStack(alignment: .leading, spacing: 6) {
                            ZStack(alignment: .topLeading) {
                                CachedImageView(
                                    url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) },
                                    contentMode: .fill,
                                    minHeight: 108,
                                    maxPixel: 480
                                )
                                .frame(height: 108)
                                .frame(maxWidth: .infinity)
                                .clipped()

                                Text(movie.number)
                                    .font(.caption2.weight(.bold))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(Color.black.opacity(0.68), in: Capsule())
                                    .padding(6)
                            }
                            .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

                            Text(movie.title ?? movie.number)
                                .font(.caption.weight(.medium))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                        }
                        .padding(7)
                        .pmlGlassCard(cornerRadius: 15)
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        Button(role: .destructive) {
                            Task {
                                await favs.toggle(.movie, id: movie.number, client: model.apiClient)
                                movies.removeAll { $0.number == movie.number }
                            }
                        } label: {
                            Label("取消收藏", systemImage: "heart.slash")
                        }
                    }
                }
            }
        }
    }

    // MARK: - 2. 收藏的演员

    @ViewBuilder
    private var performersGrid: some View {
        if performers.isEmpty {
            ContentUnavailableView("暂无收藏演员", systemImage: "person.crop.circle", description: Text("在影视库演员列表或演员详情点击爱心即可收藏。"))
                .padding(.top, 30)
        } else {
            LazyVGrid(columns: threeColumns, spacing: 12) {
                ForEach(performers) { person in
                    Button {
                        selectedPerformer = person
                    } label: {
                        VStack(spacing: 6) {
                            CachedImageView(
                                url: person.avatarUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) },
                                contentMode: .fill,
                                minHeight: 110,
                                maxPixel: 360
                            )
                            .frame(height: 110)
                            .frame(maxWidth: .infinity)
                            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                            Text(person.name)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                            Text("\(person.movieCount) 部")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        .padding(7)
                        .pmlGlassCard(cornerRadius: 14)
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        Button(role: .destructive) {
                            Task {
                                await favs.toggle(.performer, id: person.id, client: model.apiClient)
                                performers.removeAll { $0.id == person.id }
                            }
                        } label: {
                            Label("取消收藏", systemImage: "heart.slash")
                        }
                    }
                }
            }
        }
    }

    // MARK: - 3. 收藏的创作者

    @ViewBuilder
    private var creatorsGrid: some View {
        if creators.isEmpty {
            ContentUnavailableView("暂无收藏创作者", systemImage: "person.2", description: Text("在创作者大厅或创作者主页点击爱心即可收藏。"))
                .padding(.top, 30)
        } else {
            LazyVGrid(columns: twoColumns, spacing: 12) {
                ForEach(creators) { creator in
                    NavigationLink {
                        AuthorProfileView(creator: creator)
                    } label: {
                        VStack(alignment: .leading, spacing: 6) {
                            CachedImageView(
                                url: model.apiClient?.v2CoverURL(assetId: creator.bannerAssetId ?? creator.coverAssetId ?? creator.avatarAssetId),
                                contentMode: .fill,
                                minHeight: 108,
                                maxPixel: 480
                            )
                            .frame(height: 108)
                            .frame(maxWidth: .infinity)
                            .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

                            Text(creator.name)
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                            Text("\(creator.videoCount) 视频 · \(PMLFormatters.bytes(creator.totalBytes))")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        .padding(8)
                        .pmlGlassCard(cornerRadius: 15)
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        Button(role: .destructive) {
                            Task {
                                await favs.toggle(.creator, id: creator.id, client: model.apiClient)
                                creators.removeAll { $0.id == creator.id }
                            }
                        } label: {
                            Label("取消收藏", systemImage: "heart.slash")
                        }
                    }
                }
            }
        }
    }

    // MARK: - 4. 收藏的单文件

    @ViewBuilder
    private var filesGrid: some View {
        if assets.isEmpty {
            ContentUnavailableView("暂无收藏文件", systemImage: "heart", description: Text("浏览单张图片或视频时可收藏到这里。"))
                .padding(.top, 30)
        } else {
            LazyVGrid(columns: threeColumns, spacing: 10) {
                ForEach(assets) { asset in
                    VStack(alignment: .leading, spacing: 6) {
                        CachedImageView(
                            url: model.apiClient?.v2CoverURL(assetId: asset.thumbnailAssetId ?? asset.id),
                            contentMode: .fill,
                            minHeight: 104,
                            maxPixel: 360
                        )
                        .frame(height: 104)
                        .frame(maxWidth: .infinity)
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

                        Text(asset.fileName)
                            .font(.caption)
                            .lineLimit(2)
                    }
                    .onTapGesture {
                        Task { await openAsset(asset) }
                    }
                    .contextMenu {
                        Button(role: .destructive) {
                            Task { await unfavoriteAsset(asset) }
                        } label: {
                            Label("取消收藏", systemImage: "heart.slash")
                        }
                    }
                }
            }
        }
    }

    // MARK: - 数据加载与操作

    private func loadCurrentTab() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        await favs.ensureLoaded(client: client, force: true)
        do {
            switch activeTab {
            case .movies:
                let page = try await client.queryMovieLibrary(onlyFavorites: true, limit: 100, offset: 0)
                movies = page.items
            case .performers:
                let page = try await client.listMoviePerformers(onlyFavorites: true, limit: 100, offset: 0)
                performers = page.items
            case .creators:
                creators = try await client.listCreatorsV2(onlyFavorites: true)
            case .files:
                assets = try await client.favoriteAssets(limit: 200)
            }
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func openAsset(_ asset: LibraryAsset) async {
        if asset.mediaType == "video" {
            guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else { return }
            preview = PreviewItem(
                url: source.url,
                isVideo: true,
                assetId: asset.id,
                title: asset.fileName,
                fallbackURL: source.fallbackURL,
                sourceLabel: source.label,
                remotePath: asset.remotePath
            )
            return
        }
        let images = assets.filter { $0.mediaType == "image" || $0.mediaType == "comicPage" }
        let entries = images.map {
            RemoteBrowserEntry(
                path: $0.remotePath,
                name: $0.fileName,
                isDirectory: false,
                sizeBytes: $0.sizeBytes,
                modifiedAt: nil,
                etag: nil,
                assetId: $0.id,
                mediaType: $0.mediaType,
                thumbnailSource: nil,
                coverAssetId: $0.thumbnailAssetId,
                durationSeconds: $0.durationSeconds
            )
        }
        let start = images.firstIndex(where: { $0.id == asset.id }) ?? 0
        imageContext = ImageViewerContext(entries: entries, startIndex: start)
    }

    private func playAsset(assetId: String, title: String) async {
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else {
            errorText = model.playbackError ?? "无法解析视频直链"
            return
        }
        preview = PreviewItem(
            url: source.url,
            isVideo: true,
            assetId: assetId,
            title: title,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: nil
        )
    }

    private func unfavoriteAsset(_ asset: LibraryAsset) async {
        guard let client = model.apiClient else { return }
        do {
            _ = try await client.toggleFavorite(assetId: asset.id)
            assets.removeAll { $0.id == asset.id }
        } catch {
            errorText = error.localizedDescription
        }
    }
}
