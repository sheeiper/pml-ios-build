import SwiftUI
import UIKit

private enum CreatorSortV2: String, CaseIterable, Identifiable {
    case count = "作品最多"
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

// MARK: - 创作者大厅 V2

/// 创作者 V2 大厅：21 个工作室横向胶囊筛选 + 搜索姓名/别名 + 单模块独立同步 + 建议合并管理。
struct AuthorsView: View {
    @EnvironmentObject private var model: AppModel

    @State private var studios: [CreatorStudioStat] = []
    @State private var selectedStudio: String? = nil
    @State private var creators: [CreatorSummaryV2] = []
    @State private var mergeSuggestions: [CreatorMergeSuggestion] = []
    @State private var searchText = ""
    @State private var sortMode: CreatorSortV2 = .count
    @State private var loading = false
    @State private var syncing = false
    @State private var noticeText = ""
    @State private var errorText = ""
    @State private var showMergeSheet = false

    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    studioPillBar

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

                    if loading && creators.isEmpty {
                        HStack(spacing: 10) {
                            ProgressView()
                            Text("加载创作者…").foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, alignment: .center)
                        .padding(.top, 44)
                    } else if creators.isEmpty {
                        ContentUnavailableView(
                            "暂无创作者",
                            systemImage: "person.2.slash",
                            description: Text("请切换工作室筛选或点击右上角同步创作者模块。")
                        )
                        .padding(.top, 30)
                    } else {
                        HStack(alignment: .firstTextBaseline) {
                            Text(selectedStudio ?? "全部创作者")
                                .font(.headline)
                            Spacer()
                            Text("\(creators.count) 位")
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }

                        LazyVGrid(columns: columns, spacing: 12) {
                            ForEach(creators) { creator in
                                NavigationLink {
                                    AuthorProfileView(creator: creator)
                                } label: {
                                    CreatorCardV2View(creator: creator, client: model.apiClient)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
            }
            .navigationTitle("创作者")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "搜索创作者姓名或别名")
            .refreshable { await loadAll(refreshSuggestions: true) }
            .toolbar {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button {
                        showMergeSheet = true
                    } label: {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: "arrow.triangle.merge")
                            if !mergeSuggestions.isEmpty {
                                Text("\(mergeSuggestions.count)")
                                    .font(.system(size: 9, weight: .bold))
                                    .foregroundStyle(.white)
                                    .padding(.horizontal, 4)
                                    .padding(.vertical, 1)
                                    .background(Color.orange, in: Capsule())
                                    .offset(x: 7, y: -6)
                            }
                        }
                    }
                    .accessibilityLabel("建议合并")

                    Menu {
                        Picker("排序", selection: $sortMode) {
                            ForEach(CreatorSortV2.allCases) { mode in
                                Text(mode.rawValue).tag(mode)
                            }
                        }
                    } label: {
                        Image(systemName: "arrow.up.arrow.down")
                    }

                    Button {
                        Task { await syncCreatorsModule() }
                    } label: {
                        if syncing {
                            ProgressView()
                        } else {
                            Image(systemName: "arrow.clockwise")
                        }
                    }
                    .disabled(syncing)
                    .accessibilityLabel("同步创作者模块")
                }
            }
            .sheet(isPresented: $showMergeSheet) {
                CreatorMergeSuggestionsSheet(
                    suggestions: $mergeSuggestions,
                    onVerified: {
                        Task { await loadAll(refreshSuggestions: true) }
                    }
                )
            }
            .task { await loadAll(refreshSuggestions: true) }
            .onChange(of: selectedStudio) { _, _ in
                Task { await loadCreatorsOnly() }
            }
            .onChange(of: sortMode) { _, _ in
                Task { await loadCreatorsOnly() }
            }
            .onChange(of: searchText) { _, _ in
                Task { await loadCreatorsOnly() }
            }
        }
    }

    private var studioPillBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                V2FilterPill(
                    title: "全部",
                    count: studios.isEmpty ? nil : Int64(creators.count),
                    isSelected: selectedStudio == nil
                ) {
                    selectedStudio = nil
                }

                ForEach(studios) { studio in
                    V2FilterPill(
                        title: studio.studio,
                        count: studio.creatorCount,
                        isSelected: selectedStudio == studio.studio
                    ) {
                        selectedStudio = studio.studio
                    }
                }
            }
            .padding(.vertical, 2)
        }
    }

    private func loadAll(refreshSuggestions: Bool) async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            async let studioReq = client.listCreatorStudios()
            async let creatorReq = client.listCreatorsV2(
                studio: selectedStudio,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue
            )
            let (loadedStudios, loadedCreators) = try await (studioReq, creatorReq)
            studios = loadedStudios
            creators = loadedCreators
            if refreshSuggestions {
                mergeSuggestions = (try? await client.listCreatorMergeSuggestions()) ?? []
            }
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func loadCreatorsOnly() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            creators = try await client.listCreatorsV2(
                studio: selectedStudio,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue
            )
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func syncCreatorsModule() async {
        guard let client = model.apiClient, !syncing else { return }
        syncing = true
        noticeText = ""
        errorText = ""
        defer { syncing = false }
        do {
            let result = try await client.scanLibraryModule(moduleId: "creators", full: false)
            noticeText = "创作者同步完成：扫描 \(result.scan.scanned) 项，新增 \(result.scan.added)，共索引 \(result.rebuild.entityCount) 位创作者"
            await loadAll(refreshSuggestions: true)
        } catch {
            errorText = "同步失败：\(error.localizedDescription)"
        }
    }
}

// MARK: - 创作者卡片 V2

private struct CreatorCardV2View: View {
    let creator: CreatorSummaryV2
    let client: PMLAPIClient?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack(alignment: .bottomLeading) {
                CachedImageView(
                    url: client?.v2CoverURL(assetId: creator.bannerAssetId ?? creator.coverAssetId ?? creator.avatarAssetId),
                    contentMode: .fill,
                    minHeight: 112,
                    maxPixel: 480
                )
                .frame(height: 112)
                .frame(maxWidth: .infinity)
                .clipped()

                LinearGradient(
                    colors: [.clear, .black.opacity(0.68)],
                    startPoint: .center,
                    endPoint: .bottom
                )

                HStack(spacing: 8) {
                    avatarCircle
                    VStack(alignment: .leading, spacing: 2) {
                        Text(creator.name)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        if let firstAlias = creator.aliases.first, !firstAlias.isEmpty {
                            Text(firstAlias)
                                .font(.caption2)
                                .foregroundStyle(.white.opacity(0.82))
                                .lineLimit(1)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .padding(8)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Label("\(creator.videoCount)", systemImage: "film")
                    if creator.imageCount > 0 {
                        Label("\(creator.imageCount)", systemImage: "photo")
                    }
                    Spacer(minLength: 2)
                    Text(PMLFormatters.bytes(creator.totalBytes))
                        .monospacedDigit()
                }
                .font(.caption2)
                .foregroundStyle(.secondary)

                if !creator.studios.isEmpty {
                    Text(creator.studios.prefix(2).joined(separator: " · "))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 2)
        }
        .padding(8)
        .pmlGlassCard(cornerRadius: 16)
    }

    private var avatarCircle: some View {
        Group {
            if let url = client?.v2CoverURL(assetId: creator.avatarAssetId ?? creator.coverAssetId) {
                CachedImageView(url: url, contentMode: .fill, minHeight: 34, maxPixel: 160)
            } else {
                ZStack {
                    Color.accentColor.opacity(0.85)
                    Text(String(creator.name.prefix(1)))
                        .font(.caption.weight(.bold))
                        .foregroundStyle(.white)
                }
            }
        }
        .frame(width: 34, height: 34)
        .clipShape(Circle())
        .overlay(Circle().strokeBorder(.white.opacity(0.75), lineWidth: 1.2))
    }
}

// MARK: - 创作者详情页 V2（纯文件夹层级浏览 + 单视频直放）

struct AuthorProfileView: View {
    let authorId: String
    let initialName: String
    let initialCreator: CreatorSummaryV2?

    @EnvironmentObject private var model: AppModel
    @State private var creator: CreatorSummaryV2?
    @State private var selectedStudio: String? = nil
    @State private var page: FolderBrowsePage?
    @State private var loading = false
    @State private var errorText = ""
    @State private var toastText = ""
    @State private var playingVideo: V2VideoPlayPayload?
    @State private var imageViewerContext: ImageViewerContext?
    @State private var comicTarget: ComicArchiveTarget?
    @State private var showEditSheet = false

    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]

    init(creator: CreatorSummaryV2) {
        self.authorId = creator.id
        self.initialName = creator.name
        self.initialCreator = creator
    }

    /// 兼容旧调用方（ShortVideosView 等仍持有 LibraryAuthor）。
    init(author: LibraryAuthor) {
        self.authorId = author.id
        self.initialName = author.name
        self.initialCreator = nil
    }

    init(authorId: String, title: String) {
        self.authorId = authorId
        self.initialName = title
        self.initialCreator = nil
    }

    private var activeCreator: CreatorSummaryV2? {
        creator ?? initialCreator
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
                headerCard

                if let c = activeCreator, c.studios.count > 1 {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            V2FilterPill(
                                title: "全部工作室",
                                count: c.mediaCount,
                                isSelected: selectedStudio == nil
                            ) {
                                selectedStudio = nil
                            }
                            ForEach(c.studios, id: \.self) { studio in
                                V2FilterPill(
                                    title: studio,
                                    count: c.studioCounts[studio],
                                    isSelected: selectedStudio == studio
                                ) {
                                    selectedStudio = studio
                                }
                            }
                        }
                    }
                }

                if !toastText.isEmpty {
                    Text(toastText)
                        .font(.footnote)
                        .foregroundStyle(.green)
                }

                if !errorText.isEmpty {
                    Text(errorText)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }

                if loading && page == nil {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("加载作品目录…").foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 30)
                } else if let page {
                    if !page.folders.isEmpty {
                        folderSection(page.folders)
                    }
                    if !page.files.isEmpty {
                        fileSection(page.files)
                    }
                    if page.folders.isEmpty && page.files.isEmpty {
                        ContentUnavailableView(
                            "暂无内容",
                            systemImage: "folder",
                            description: Text("当前筛选下没有文件夹或文件。")
                        )
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .navigationTitle(activeCreator?.name ?? initialName)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await loadFolder() }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showEditSheet = true
                } label: {
                    Image(systemName: "pencil")
                }
                .disabled(activeCreator == nil)
                .accessibilityLabel("编辑创作者资料")
            }
        }
        .sheet(isPresented: $showEditSheet) {
            if let current = activeCreator {
                EditCreatorProfileSheet(creator: current) { updated in
                    creator = updated
                }
            }
        }
        .fullScreenCover(item: $playingVideo) { payload in
            GestureVideoPlayerSheet(
                url: payload.url,
                title: payload.title,
                assetId: payload.assetId,
                playlist: payload.playlist,
                playlistIndex: payload.playlistIndex,
                remotePath: payload.remotePath
            )
        }
        .fullScreenCover(item: $imageViewerContext) { ctx in
            ImageViewer(context: ctx)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target)
        }
        .task(id: authorId) { await loadFolder() }
        .onChange(of: selectedStudio) { _, _ in
            Task { await loadFolder() }
        }
    }

    private var headerCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            ZStack(alignment: .bottomLeading) {
                CachedImageView(
                    url: model.apiClient?.v2CoverURL(assetId: activeCreator?.bannerAssetId ?? activeCreator?.coverAssetId),
                    contentMode: .fill,
                    minHeight: 150,
                    maxPixel: 960
                )
                .frame(height: 150)
                .frame(maxWidth: .infinity)
                .clipped()

                LinearGradient(
                    colors: [.clear, .black.opacity(0.74)],
                    startPoint: .top,
                    endPoint: .bottom
                )

                HStack(alignment: .bottom, spacing: 12) {
                    Group {
                        if let url = model.apiClient?.v2CoverURL(assetId: activeCreator?.avatarAssetId ?? activeCreator?.coverAssetId) {
                            CachedImageView(url: url, contentMode: .fill, minHeight: 58, maxPixel: 240)
                        } else {
                            ZStack {
                                Color.accentColor
                                Text(String((activeCreator?.name ?? initialName).prefix(1)))
                                    .font(.title2.weight(.bold))
                                    .foregroundStyle(.white)
                            }
                        }
                    }
                    .frame(width: 58, height: 58)
                    .clipShape(Circle())
                    .overlay(Circle().strokeBorder(.white.opacity(0.85), lineWidth: 2))

                    VStack(alignment: .leading, spacing: 3) {
                        Text(activeCreator?.name ?? initialName)
                            .font(.title3.weight(.bold))
                            .foregroundStyle(.white)
                        if let aliases = activeCreator?.aliases, !aliases.isEmpty {
                            Text("别名：" + aliases.joined(separator: " / "))
                                .font(.caption)
                                .foregroundStyle(.white.opacity(0.85))
                                .lineLimit(2)
                        }
                        if let c = activeCreator {
                            Text("\(c.videoCount) 视频 · \(c.imageCount) 图片 · \(PMLFormatters.bytes(c.totalBytes))")
                                .font(.caption2.monospacedDigit())
                                .foregroundStyle(.white.opacity(0.8))
                        }
                    }
                    Spacer()
                }
                .padding(12)
            }
            .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))

            if let bio = activeCreator?.bio, !bio.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(bio)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            }

            if let links = activeCreator?.socialLinks, !links.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(links.keys.sorted(), id: \.self) { key in
                            if let raw = links[key], !raw.isEmpty,
                               let url = URL(string: raw.hasPrefix("http") ? raw : "https://\(raw)") {
                                Link(destination: url) {
                                    Label(key, systemImage: "link")
                                        .font(.caption.weight(.medium))
                                        .padding(.horizontal, 10)
                                        .padding(.vertical, 6)
                                        .background(Color.accentColor.opacity(0.14), in: Capsule())
                                }
                            }
                        }
                    }
                }
            }
        }
        .padding(10)
        .pmlGlassCard(cornerRadius: 20)
    }

    private func folderSection(_ folders: [FolderCardV2]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("文件夹（\(folders.count)）")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.secondary)

            LazyVGrid(columns: columns, spacing: 12) {
                ForEach(folders) { folder in
                    if folder.isSingleVideoWrapper, let singleId = folder.singleVideoAssetId {
                        Button {
                            Task { await playSingleVideo(assetId: singleId, title: folder.name) }
                        } label: {
                            V2FolderCardView(
                                title: folder.name,
                                subtitle: folder.studioName.isEmpty ? nil : folder.studioName,
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
                            CreatorFolderView(
                                authorId: authorId,
                                creatorName: activeCreator?.name ?? initialName,
                                studio: selectedStudio ?? (folder.studioName.isEmpty ? nil : folder.studioName),
                                subPath: folder.subPath,
                                folderTitle: folder.name
                            )
                        } label: {
                            V2FolderCardView(
                                title: folder.name,
                                subtitle: folder.studioName.isEmpty ? nil : folder.studioName,
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

    private func fileSection(_ files: [CreatorAssetV2]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("文件（\(files.count)）")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.secondary)

            LazyVGrid(columns: columns, spacing: 12) {
                ForEach(files) { asset in
                    Button {
                        Task { await openAsset(asset, siblings: files) }
                    } label: {
                        V2AssetCardView(asset: asset, client: model.apiClient)
                    }
                    .buttonStyle(.plain)
                    .contextMenu {
                        Button {
                            Task { await setAuthorImage(assetId: asset.id, isBanner: false) }
                        } label: {
                            Label("设为创作者头像", systemImage: "person.crop.circle")
                        }
                        Button {
                            Task { await setAuthorImage(assetId: asset.id, isBanner: true) }
                        } label: {
                            Label("设为创作者横幅", systemImage: "photo.artframe")
                        }
                    }
                }
            }
        }
    }

    private func loadFolder() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            let resp = try await client.browseCreatorFolder(
                authorId: authorId,
                studio: selectedStudio,
                subPath: nil,
                limit: 200
            )
            creator = resp.creator
            page = resp.page
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func playSingleVideo(assetId: String, title: String) async {
        guard let client = model.apiClient else { return }
        do {
            let url = try await client.mediaSourceURL(assetId: assetId)
            playingVideo = V2VideoPlayPayload(
                url: url,
                title: title,
                assetId: assetId,
                playlist: [VideoPlaylistItem(id: assetId, title: title, assetId: assetId)],
                playlistIndex: 0,
                remotePath: nil
            )
        } catch {
            errorText = "打开视频失败：\(error.localizedDescription)"
        }
    }

    private func openAsset(_ asset: CreatorAssetV2, siblings: [CreatorAssetV2]) async {
        guard let client = model.apiClient else { return }
        if asset.isVideo {
            let videos = siblings.filter(\.isVideo)
            let playlist = videos.map {
                VideoPlaylistItem(id: $0.id, title: $0.fileName, assetId: $0.id, remotePath: $0.remotePath)
            }
            let idx = videos.firstIndex(where: { $0.id == asset.id })
            do {
                let url = try await client.mediaSourceURL(assetId: asset.id)
                playingVideo = V2VideoPlayPayload(
                    url: url,
                    title: asset.fileName,
                    assetId: asset.id,
                    playlist: playlist,
                    playlistIndex: idx,
                    remotePath: asset.remotePath
                )
            } catch {
                errorText = "打开视频失败：\(error.localizedDescription)"
            }
        } else if asset.isComicArchive {
            comicTarget = ComicArchiveTarget(id: asset.id, title: asset.fileName)
        } else if asset.isImage {
            let images = siblings.filter(\.isImage)
            let entries = images.map { $0.toRemoteBrowserEntry() }
            let startIdx = images.firstIndex(where: { $0.id == asset.id }) ?? 0
            imageViewerContext = ImageViewerContext(entries: entries, startIndex: startIdx)
        }
    }

    private func setAuthorImage(assetId: String, isBanner: Bool) async {
        guard let client = model.apiClient else { return }
        do {
            if isBanner {
                try await client.setAuthorBanner(authorId: authorId, assetId: assetId)
                toastText = "已设为创作者横幅"
            } else {
                try await client.setAuthorAvatar(authorId: authorId, assetId: assetId)
                toastText = "已设为创作者头像"
            }
            await loadFolder()
        } catch {
            errorText = error.localizedDescription
        }
    }
}

// MARK: - 创作者子目录递归视图

struct CreatorFolderView: View {
    let authorId: String
    let creatorName: String
    let studio: String?
    let subPath: String
    let folderTitle: String

    @EnvironmentObject private var model: AppModel
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
                        Text("加载目录…").foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
                } else if let page {
                    if !page.breadcrumbs.isEmpty {
                        Text(page.breadcrumbs.joined(separator: " / "))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    if !page.folders.isEmpty {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("子文件夹（\(page.folders.count)）")
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
                                            CreatorFolderView(
                                                authorId: authorId,
                                                creatorName: creatorName,
                                                studio: studio,
                                                subPath: folder.subPath,
                                                folderTitle: folder.name
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
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        .navigationTitle(folderTitle)
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .fullScreenCover(item: $playingVideo) { payload in
            GestureVideoPlayerSheet(
                url: payload.url,
                title: payload.title,
                assetId: payload.assetId,
                playlist: payload.playlist,
                playlistIndex: payload.playlistIndex,
                remotePath: payload.remotePath
            )
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
            let resp = try await client.browseCreatorFolder(
                authorId: authorId,
                studio: studio,
                subPath: subPath,
                limit: 300
            )
            page = resp.page
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func playSingleVideo(assetId: String, title: String) async {
        guard let client = model.apiClient else { return }
        do {
            let url = try await client.mediaSourceURL(assetId: assetId)
            playingVideo = V2VideoPlayPayload(
                url: url,
                title: title,
                assetId: assetId,
                playlist: [VideoPlaylistItem(id: assetId, title: title, assetId: assetId)],
                playlistIndex: 0,
                remotePath: nil
            )
        } catch {
            errorText = "打开视频失败：\(error.localizedDescription)"
        }
    }

    private func openAsset(_ asset: CreatorAssetV2, siblings: [CreatorAssetV2]) async {
        guard let client = model.apiClient else { return }
        if asset.isVideo {
            let videos = siblings.filter(\.isVideo)
            let playlist = videos.map {
                VideoPlaylistItem(id: $0.id, title: $0.fileName, assetId: $0.id, remotePath: $0.remotePath)
            }
            let idx = videos.firstIndex(where: { $0.id == asset.id })
            do {
                let url = try await client.mediaSourceURL(assetId: asset.id)
                playingVideo = V2VideoPlayPayload(
                    url: url,
                    title: asset.fileName,
                    assetId: asset.id,
                    playlist: playlist,
                    playlistIndex: idx,
                    remotePath: asset.remotePath
                )
            } catch {
                errorText = "打开视频失败：\(error.localizedDescription)"
            }
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

// MARK: - 编辑创作者资料 Sheet

private struct EditCreatorProfileSheet: View {
    let creator: CreatorSummaryV2
    let onSaved: (CreatorSummaryV2) -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var aliasesText: String
    @State private var bioText: String
    @State private var twitter: String
    @State private var onlyfans: String
    @State private var website: String
    @State private var saving = false
    @State private var errorText = ""

    init(creator: CreatorSummaryV2, onSaved: @escaping (CreatorSummaryV2) -> Void) {
        self.creator = creator
        self.onSaved = onSaved
        _aliasesText = State(initialValue: creator.aliases.joined(separator: ", "))
        _bioText = State(initialValue: creator.bio ?? "")
        _twitter = State(initialValue: creator.socialLinks["twitter"] ?? creator.socialLinks["X"] ?? "")
        _onlyfans = State(initialValue: creator.socialLinks["onlyfans"] ?? "")
        _website = State(initialValue: creator.socialLinks["website"] ?? "")
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("基本信息") {
                    TextField("别名（逗号分隔）", text: $aliasesText)
                    TextField("个人简介", text: $bioText, axis: .vertical)
                        .lineLimit(3...6)
                }
                Section("社交链接") {
                    TextField("Twitter / X 链接", text: $twitter)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                    TextField("OnlyFans 链接", text: $onlyfans)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                    TextField("主页 / 其他链接", text: $website)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                }
                if !errorText.isEmpty {
                    Section {
                        Text(errorText).foregroundStyle(.red).font(.footnote)
                    }
                }
            }
            .navigationTitle("编辑资料")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(saving ? "保存中…" : "保存") {
                        Task { await save() }
                    }
                    .disabled(saving)
                }
            }
        }
    }

    private func save() async {
        guard let client = model.apiClient else { return }
        saving = true
        errorText = ""
        defer { saving = false }
        let aliases = aliasesText
            .split(whereSeparator: { $0 == "," || $0 == "，" || $0 == "\n" })
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        var links = creator.socialLinks
        let pairs: [(String, String)] = [("twitter", twitter), ("onlyfans", onlyfans), ("website", website)]
        for (key, value) in pairs {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty {
                links.removeValue(forKey: key)
            } else {
                links[key] = trimmed
            }
        }
        do {
            let updated = try await client.updateCreatorProfile(
                authorId: creator.id,
                aliases: aliases,
                bio: bioText.trimmingCharacters(in: .whitespacesAndNewlines),
                socialLinks: links
            )
            onSaved(updated)
            dismiss()
        } catch {
            errorText = error.localizedDescription
        }
    }
}

// MARK: - 创作者建议合并 Sheet（手动去 115 合并 + 已合并复查 + 忽略）

private struct CreatorMergeSuggestionsSheet: View {
    @Binding var suggestions: [CreatorMergeSuggestion]
    let onVerified: () -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var busyPairId: String?
    @State private var keepAlias = true
    @State private var statusMessage = ""
    @State private var errorText = ""

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Toggle("不同名合并时保留旧名为别名", isOn: $keepAlias)
                    Text("在 115 网盘手动将重复创作者的文件夹合并后，点击「已合并，复查」即可自动重扫并保留别名。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                if !statusMessage.isEmpty {
                    Section {
                        Text(statusMessage)
                            .font(.footnote)
                            .foregroundStyle(.green)
                    }
                }

                if !errorText.isEmpty {
                    Section {
                        Text(errorText)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                }

                if suggestions.isEmpty {
                    ContentUnavailableView(
                        "没有疑似重复创作者",
                        systemImage: "checkmark.seal.fill",
                        description: Text("所有创作者名称都很干净。")
                    )
                } else {
                    ForEach(suggestions) { item in
                        VStack(alignment: .leading, spacing: 10) {
                            HStack {
                                Text(item.reason)
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.orange)
                                Spacer()
                                Text("相似度 \(item.score)")
                                    .font(.caption2.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }

                            candidateRow(item.authorA, tag: "A")
                            candidateRow(item.authorB, tag: "B")

                            HStack(spacing: 8) {
                                Button {
                                    Task { await verify(keep: item.authorA, merged: item.authorB, pairId: item.id) }
                                } label: {
                                    Text("保留 A 复查")
                                        .font(.caption.weight(.semibold))
                                }
                                .buttonStyle(.borderedProminent)
                                .disabled(busyPairId != nil)

                                Button {
                                    Task { await verify(keep: item.authorB, merged: item.authorA, pairId: item.id) }
                                } label: {
                                    Text("保留 B 复查")
                                        .font(.caption.weight(.semibold))
                                }
                                .buttonStyle(.bordered)
                                .disabled(busyPairId != nil)

                                Spacer()

                                Button("忽略", role: .destructive) {
                                    Task { await ignore(item) }
                                }
                                .font(.caption)
                                .disabled(busyPairId != nil)
                            }
                        }
                        .padding(.vertical, 6)
                    }
                }
            }
            .navigationTitle("建议合并（\(suggestions.count)）")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成") { dismiss() }
                }
            }
        }
    }

    private func candidateRow(_ author: MergeCandidateAuthor, tag: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(tag)
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(tag == "A" ? Color.accentColor : Color.purple, in: Capsule())
                Text(author.name)
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text("\(author.mediaCount) 项 · \(PMLFormatters.bytes(author.totalBytes))")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            if let path = author.paths.first {
                Button {
                    UIPasteboard.general.string = path
                    statusMessage = "已复制路径：\(path)"
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "doc.on.doc")
                            .font(.caption2)
                        Text(path)
                            .font(.caption2)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
    }

    private func verify(keep: MergeCandidateAuthor, merged: MergeCandidateAuthor, pairId: String) async {
        guard let client = model.apiClient else { return }
        busyPairId = pairId
        errorText = ""
        statusMessage = ""
        defer { busyPairId = nil }
        do {
            let result = try await client.verifyCreatorMerge(
                keepAuthorId: keep.id,
                mergedAuthorId: merged.id,
                keepAlias: keepAlias
            )
            if result.fullyMerged {
                statusMessage = "已确认合并到「\(result.keepAuthorName)」，共 \(result.keepTotalAssets) 项。"
                suggestions.removeAll { $0.id == pairId }
                onVerified()
            } else {
                statusMessage = "「\(result.mergedAuthorName)」下仍有 \(result.remainingOldAssets) 项未移完，请先在 115 移完再同步复查。"
            }
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func ignore(_ item: CreatorMergeSuggestion) async {
        guard let client = model.apiClient else { return }
        busyPairId = item.id
        errorText = ""
        defer { busyPairId = nil }
        do {
            try await client.ignoreCreatorMergePair(authorIdA: item.authorA.id, authorIdB: item.authorB.id)
            suggestions.removeAll { $0.id == item.id }
        } catch {
            errorText = error.localizedDescription
        }
    }
}
