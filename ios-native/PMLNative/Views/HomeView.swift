import SwiftUI
import UIKit

/// 首页：资料库概览、最近新增、扫描入口。
struct HomeView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase

    @State private var overview: LibraryOverview?
    @State private var recent: [LibraryAsset] = []
    @State private var authors: [LibraryAuthor] = []
    @State private var comics: [LibraryAsset] = []
    @State private var roots: [LibraryRoot] = []
    @State private var favorites: Int64 = 0
    @State private var loading = false
    @State private var errorText = ""
    @State private var scanMessage = ""
    @State private var scanning = false
    @State private var preview: PreviewItem?
    @State private var archiveComic: ComicArchiveTarget?
    @State private var folderComic: ComicFolderTarget?
    @State private var directoryTarget: HomeDirectoryTarget?
    @StateObject private var thumbs = ThumbnailLoader()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    if let overview {
                        statsGrid(overview)
                    } else if loading {
                        ProgressView().frame(maxWidth: .infinity, alignment: .center)
                    }

                    if !errorText.isEmpty {
                        Text(errorText)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }

                    recentSection
                    creatorsSection
                    comicsSection
                    scanCard
                }
                .padding(20)
            }
            .navigationTitle("首页")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 16) {
                        NavigationLink {
                            FavoritesView()
                        } label: {
                            Image(systemName: "heart")
                        }
                        NavigationLink {
                            HistoryView()
                        } label: {
                            Image(systemName: "clock.arrow.circlepath")
                        }
                        NavigationLink {
                            SettingsView()
                        } label: {
                            Image(systemName: "gearshape")
                        }
                    }
                }
            }
'            .task { await refresh() }
            .onChange(of: scenePhase) { _, phase in
                guard phase == .active else { return }
                if errorText.lowercased().contains("cancel") || overview == nil {
                    Task { await refresh() }
                }
            }'
            .refreshable { await refresh() }
            .fullScreenCover(item: $preview) { item in
                if item.isVideo {
                    GestureVideoPlayerSheet(
                        url: item.url,
                        title: item.title,
                        assetId: item.assetId,
                        startAt: item.startAt, fallbackURL: item.fallbackURL, sourceLabel: item.sourceLabel
                    )
                    .environmentObject(model)
                } else {
                    ImageViewerSheet(url: item.url)
                }
            }
            .fullScreenCover(item: $archiveComic) { target in
                ComicReaderView(target: target).environmentObject(model)
            }
            .fullScreenCover(item: $folderComic) { target in
                ComicFolderReaderView(target: target).environmentObject(model)
            }
            .sheet(item: $directoryTarget) { target in
                NavigationStack {
                    DirectoryView(root: target.root, path: target.path)
                        .environmentObject(model)
                }
            }
        }
    }

    private func statsGrid(_ overview: LibraryOverview) -> some View {
        LazyVGrid(columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)], spacing: 12) {
            statCard("媒体", "\(overview.assets)", "photo.on.rectangle.angled")
            statCard("图片", "\(overview.images)", "photo.fill")
            statCard("视频", "\(overview.videos)", "play.rectangle.fill")
            statCard("漫画", "\(overview.comics)", "book.closed.fill")
            statCard("创作者", "\(overview.authors)", "person.2.fill")
            statCard("收藏", "\(favorites)", "heart.fill")
            statCard("未播放", "\(overview.unplayed)", "eye.slash")
            statCard("占用", pmlFormatBytes(overview.totalBytes), "internaldrive")
        }
    }

    private func statCard(_ title: String, _ value: String, _ symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: symbol)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.tint)
                Text(title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(value)
                .font(.title3.weight(.semibold))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .pmlGlassCard(cornerRadius: 18)
    }

    @ViewBuilder
    private var recentSection: some View {
        if !recent.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("最近新增")
                    .font(.headline)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(recent) { asset in
                            Button {
                                Task { await open(asset) }
                            } label: {
                                recentCell(asset)
                            }
                            .buttonStyle(.plain)
                            .task {
                                await thumbs.image(assetId: asset.thumbnailAssetId ?? asset.id, client: model.apiClient)
                            }
                        }
                    }
                }
            }
        }
    }

    private func recentCell(_ asset: LibraryAsset) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(.quaternary.opacity(0.35))
                if let image = thumbs.images[asset.thumbnailAssetId ?? asset.id] {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: asset.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                        .font(.system(size: 20))
                        .foregroundStyle(.tint)
                }
            }
            .frame(width: 132, height: 96)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            Text(asset.fileName)
                .font(.caption)
                .lineLimit(1)
                .frame(width: 132, alignment: .leading)
        }
    }

    private var scanCard: some View {
        NavigationLink {
            ScanCenterView()
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label("资料库扫描", systemImage: "arrow.triangle.2.circlepath")
                        .font(.headline)
                    Spacer()
                    Text("去扫描").font(.caption).foregroundStyle(.tint)
                }
                Text("扫描范围：已整理、漫画（最近添加与待整理不扫描）")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(16)
            .pmlGlassCard(cornerRadius: 20)
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var creatorsSection: some View {
        if !authors.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("常看创作者").font(.headline)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 14) {
                        ForEach(authors.prefix(10)) { author in
                            NavigationLink {
                                AuthorProfileView(author: author)
                                    .environmentObject(model)
                            } label: {
                                VStack(spacing: 6) {
                                    avatarView(author)
                                        .frame(width: 60, height: 60)
                                        .clipShape(Circle())
                                    Text(author.name)
                                        .font(.caption)
                                        .lineLimit(1)
                                        .frame(width: 68)
                                }
                            }
                            .buttonStyle(.plain)
                            .task {
                                if let id = author.avatarAssetId {
                                    await thumbs.image(assetId: id, client: model.apiClient)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var comicsSection: some View {
        if !comics.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                Text("漫画馆藏").font(.headline)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(comics) { asset in
                            Button {
                                Task { await openComic(asset) }
                            } label: {
                                comicCell(asset)
                            }
                            .buttonStyle(.plain)
                            .task {
                                await thumbs.image(assetId: asset.thumbnailAssetId ?? asset.id, client: model.apiClient)
                            }
                        }
                    }
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

    private func comicCell(_ asset: LibraryAsset) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(.quaternary.opacity(0.35))
                if let image = thumbs.images[asset.thumbnailAssetId ?? asset.id] {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: "book.closed.fill")
                        .font(.system(size: 22))
                        .foregroundStyle(.tint)
                }
            }
            .frame(width: 116, height: 154)
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            Text(asset.fileName)
                .font(.caption)
                .lineLimit(1)
                .frame(width: 116, alignment: .leading)
        }
    }

    private func openComic(_ asset: LibraryAsset) async {
        let lower = asset.fileName.lowercased()
        if lower.hasSuffix(".cbz") || lower.hasSuffix(".zip") || lower.hasSuffix(".cbr") || lower.hasSuffix(".rar") {
            archiveComic = ComicArchiveTarget(id: asset.id, title: asset.fileName)
            return
        }
        guard let root = roots.first(where: { $0.id == asset.rootId }) else { return }
        directoryTarget = HomeDirectoryTarget(
            root: root,
            path: asset.parentPath.isEmpty ? root.remotePath : asset.parentPath
        )
    }

    private func refresh(retryOnCancel: Bool = true) async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            async let overviewTask = client.libraryOverview()
            async let recentTask = client.libraryAssets(filter: "recent", limit: 14)
            async let favoriteTask = client.favoriteCount()
            async let authorsTask = client.libraryAuthors(filter: "all")
            async let comicsTask = client.libraryAssets(filter: "comics", limit: 16)
            async let rootsTask = client.libraryRoots()
            overview = try await overviewTask
            recent = try await recentTask
            favorites = try await favoriteTask
            authors = try await authorsTask
            comics = try await comicsTask
            roots = try await rootsTask
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
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

    private func startScan() async {
        guard let client = model.apiClient, !scanning else { return }
        scanning = true
        scanMessage = "开始扫描…"
        defer { scanning = false }
        do {
            try await client.startScan()
            scanMessage = "扫描中…"
            for _ in 0..<900 {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard let progress = try? await client.scanProgress() else { break }
                scanMessage = "\(progress.message) \(Int(progress.percent))%"
                if !progress.running {
                    scanMessage = progress.error ?? "扫描完成：索引 \(progress.filesIndexed) 项"
                    break
                }
            }
            await refresh()
        } catch {
            scanMessage = error.localizedDescription
        }
    }
}

struct HomeDirectoryTarget: Identifiable {
    let id = UUID()
    let root: LibraryRoot
    let path: String
}
