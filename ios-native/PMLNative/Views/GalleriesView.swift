import SwiftUI

private enum GallerySortV2: String, CaseIterable, Identifiable {
    case latest = "最新入库"
    case count = "页数最多"
    case size = "体积最大"
    case name = "名称 A-Z"
    case random = "随机漫步"

    var id: String { rawValue }
    var apiValue: String {
        switch self {
        case .latest: return "latest"
        case .count: return "count"
        case .size: return "size"
        case .name: return "name"
        case .random: return "random"
        }
    }
}

private enum GalleryKindFilter: String, CaseIterable, Identifiable {
    case all = "全部"
    case archive = "压缩包"
    case folder = "文件夹"

    var id: String { rawValue }
    var apiValue: String {
        switch self {
        case .all: return "all"
        case .archive: return "archive"
        case .folder: return "folder"
        }
    }
}

private struct GalleryReaderTarget: Identifiable {
    let id = UUID()
    let title: String
    let sources: [ReaderPageSource]
}

// MARK: - 写真图集大厅 V2

/// 写真图集 V2：筛选/类型/排序/随机统一收纳右上角 + 3:4 竖版卡片 + 长按更换封面 + 文件夹/ZIP统一新阅读器。
struct GalleriesView: View {
    @EnvironmentObject private var model: AppModel

    @State private var models: [GalleryModelStat] = []
    @State private var selectedModel: String? = nil
    @State private var kindFilter: GalleryKindFilter = .all
    @State private var sets: [GallerySetV2] = []
    @State private var totalSets: Int64 = 0
    @State private var searchText = ""
    @State private var sortMode: GallerySortV2 = .latest
    @State private var showFilterSheet = false
    @State private var loading = false
    @State private var loadingMore = false
    @State private var syncing = false
    @State private var openingSetId: String? = nil
    @State private var noticeText = ""
    @State private var errorText = ""
    @State private var galleryReaderTarget: GalleryReaderTarget?
    @State private var comicTarget: ComicArchiveTarget?
    @State private var coverPickerSet: GallerySetV2?

    private let pageSize = 60
    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]

    private var hasActiveFilters: Bool {
        selectedModel != nil || sortMode != .latest || kindFilter != .all
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
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

                if loading && sets.isEmpty {
                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(0..<8, id: \.self) { _ in
                            VStack(alignment: .leading, spacing: 8) {
                                PMLSkeletonBlock(cornerRadius: 12)
                                    .aspectRatio(3.0 / 4.0, contentMode: .fit)
                                PMLSkeletonBlock(cornerRadius: 4)
                                    .frame(height: 13)
                                PMLSkeletonBlock(cornerRadius: 4)
                                    .frame(width: 90, height: 10)
                            }
                            .padding(8)
                            .pmlGlassCard(cornerRadius: 16)
                        }
                    }
                } else if sets.isEmpty {
                    ContentUnavailableView(
                        "暂无图集",
                        systemImage: "photo.on.rectangle.angled",
                        description: Text("请切换模特或点击右上角同步图集模块。")
                    )
                    .padding(.top, 30)
                } else {
                    HStack(alignment: .firstTextBaseline) {
                        Text(selectedModel ?? "全部图集")
                            .font(.headline)
                        if kindFilter != .all {
                            Text(kindFilter.rawValue)
                                .font(.caption2.weight(.semibold))
                                .padding(.horizontal, 7)
                                .padding(.vertical, 2)
                                .background(Color.purple.opacity(0.18), in: Capsule())
                                .foregroundStyle(.purple)
                        }
                        Spacer()
                        Text("\(totalSets) 套")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }

                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(sets) { item in
                            Button {
                                Task { await openGallerySet(item) }
                            } label: {
                                GallerySetCardView(
                                    item: item,
                                    isOpening: openingSetId == item.id,
                                    client: model.apiClient
                                )
                            }
                            .buttonStyle(.plain)
                            .disabled(openingSetId != nil)
                            .contextMenu {
                                Button {
                                    coverPickerSet = item
                                } label: {
                                    Label("更换封面", systemImage: "photo.on.rectangle")
                                }
                                Button {
                                    Task { await openGallerySet(item) }
                                } label: {
                                    Label("打开图集", systemImage: "eye")
                                }
                            }
                        }
                    }

                    if Int64(sets.count) < totalSets {
                        Button {
                            Task { await loadMore() }
                        } label: {
                            HStack(spacing: 8) {
                                if loadingMore { ProgressView() }
                                Text(loadingMore ? "加载中…" : "加载更多（已显示 \(sets.count)/\(totalSets)）")
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
        .navigationTitle("写真图集")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "搜索图集或模特名称")
        .refreshable { await loadAll() }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showFilterSheet = true
                } label: {
                    Image(systemName: hasActiveFilters ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                }
                .accessibilityLabel("图集筛选与排序")
            }
        }
        .sheet(isPresented: $showFilterSheet) {
            NavigationStack {
                Form {
                    Section("图集类型") {
                        Picker("类型", selection: $kindFilter) {
                            ForEach(GalleryKindFilter.allCases) { k in
                                Text(k.rawValue).tag(k)
                            }
                        }
                        .pickerStyle(.segmented)
                    }
                    Section("排序与随机") {
                        Picker("排序", selection: $sortMode) {
                            ForEach(GallerySortV2.allCases) { mode in
                                Text(mode.rawValue).tag(mode)
                            }
                        }
                        Button {
                            sortMode = .random
                            showFilterSheet = false
                            Task { await loadSetsOnly() }
                        } label: {
                            Label("随机漫步（换一批随机顺序）", systemImage: "shuffle")
                        }
                    }
                    Section("按模特筛选") {
                        Button {
                            selectedModel = nil
                            showFilterSheet = false
                        } label: {
                            HStack {
                                Text("全部模特")
                                Spacer()
                                if selectedModel == nil { Image(systemName: "checkmark").foregroundStyle(.blue) }
                            }
                        }
                        ForEach(models) { m in
                            Button {
                                selectedModel = m.modelName
                                showFilterSheet = false
                            } label: {
                                HStack {
                                    Text(m.modelName)
                                    Spacer()
                                    Text("\(m.setCount) 套").font(.caption).foregroundStyle(.secondary)
                                    if selectedModel == m.modelName { Image(systemName: "checkmark").foregroundStyle(.blue) }
                                }
                            }
                        }
                    }
                    Section {
                        Button {
                            showFilterSheet = false
                            Task { await syncGalleriesModule() }
                        } label: {
                            Label(syncing ? "同步中…" : "同步写真图集", systemImage: "arrow.clockwise")
                        }
                        .disabled(syncing)

                        if hasActiveFilters {
                            Button("重置筛选", role: .destructive) {
                                selectedModel = nil
                                kindFilter = .all
                                sortMode = .latest
                                showFilterSheet = false
                            }
                        }
                    }
                }
                .navigationTitle("图集筛选与排序")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) { Button("完成") { showFilterSheet = false } }
                }
            }
            .presentationDetents([.medium, .large])
        }
        .sheet(item: $coverPickerSet) { setItem in
            GalleryCoverPickerSheet(set: setItem) { updated in
                if let idx = sets.firstIndex(where: { $0.id == updated.id }) {
                    sets[idx] = updated
                }
                noticeText = "已更新「\(updated.albumName)」封面"
            }
            .environmentObject(model)
        }
        .fullScreenCover(item: $galleryReaderTarget) { target in
            ComicGalleryReaderView(kind: .gallery, title: target.title, sources: target.sources)
                .environmentObject(model)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target, kind: .gallery)
                .environmentObject(model)
        }
        .task { await loadAll() }
        .onChange(of: selectedModel) { _, _ in
            Task { await loadSetsOnly() }
        }
        .onChange(of: kindFilter) { _, _ in
            Task { await loadSetsOnly() }
        }
        .onChange(of: sortMode) { _, _ in
            Task { await loadSetsOnly() }
        }
        .onChange(of: searchText) { _, _ in
            Task { await loadSetsOnly() }
        }
    }

    private func loadAll() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            async let modelReq = client.listGalleryModels()
            async let setsReq = client.listGallerySets(
                modelName: selectedModel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
                kindFilter: kindFilter.apiValue,
                limit: pageSize,
                offset: 0
            )
            let (loadedModels, page) = try await (modelReq, setsReq)
            models = loadedModels
            sets = page.sets
            totalSets = page.total
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func loadSetsOnly() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            let page = try await client.listGallerySets(
                modelName: selectedModel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
                kindFilter: kindFilter.apiValue,
                limit: pageSize,
                offset: 0
            )
            sets = page.sets
            totalSets = page.total
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func loadMore() async {
        guard let client = model.apiClient, !loadingMore else { return }
        loadingMore = true
        defer { loadingMore = false }
        do {
            let page = try await client.listGallerySets(
                modelName: selectedModel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
                kindFilter: kindFilter.apiValue,
                limit: pageSize,
                offset: sets.count
            )
            sets.append(contentsOf: page.sets)
            totalSets = page.total
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func syncGalleriesModule() async {
        guard let client = model.apiClient, !syncing else { return }
        let prevTotal = totalSets
        syncing = true
        noticeText = ""
        errorText = ""
        defer { syncing = false }
        do {
            let result = try await client.scanLibraryModule(moduleId: "galleries", full: false)
            let delta = result.rebuild.entityCount - prevTotal
            let deltaStr = delta > 0 ? "（新增 +\(delta) 套）" : (delta < 0 ? "（减少 \(delta) 套）" : "（无新增套图）")
            noticeText = "图集同步完成：新增文件 +\(result.scan.added) · 更新 \(result.scan.updated) · 缺失 \(result.scan.missing)，共索引 \(result.rebuild.entityCount) 套\(deltaStr)"
            await loadAll()
        } catch {
            errorText = "同步失败：\(error.localizedDescription)"
        }
    }

    private func openGallerySet(_ item: GallerySetV2) async {
        guard let client = model.apiClient else { return }
        if item.isArchive, let coverId = item.coverAssetId, !coverId.isEmpty {
            comicTarget = ComicArchiveTarget(id: coverId, title: item.albumName)
            return
        }
        openingSetId = item.id
        errorText = ""
        defer { openingSetId = nil }
        do {
            let assets = try await client.getGallerySetItems(setId: item.id)
            if item.isArchive, let archive = assets.first {
                comicTarget = ComicArchiveTarget(id: archive.id, title: item.albumName)
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
                    galleryReaderTarget = GalleryReaderTarget(title: item.albumName, sources: sources)
                } else {
                    errorText = "无法解析该图集图片地址。"
                }
            } else if let archive = assets.first(where: \.isComicArchive) {
                comicTarget = ComicArchiveTarget(id: archive.id, title: item.albumName)
            } else {
                errorText = "该图集下未找到可浏览的图片。"
            }
        } catch {
            errorText = "打开图集失败：\(error.localizedDescription)"
        }
    }
}

private struct GallerySetCardView: View {
    let item: GallerySetV2
    let isOpening: Bool
    let client: PMLAPIClient?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack {
                Color.secondary.opacity(0.12)
                    .aspectRatio(3.0 / 4.0, contentMode: .fit)
                    .overlay {
                        CachedImageView(
                            url: client?.v2CoverURL(
                                assetId: item.coverAssetId,
                                isArchive: item.isArchive,
                                pageIndex: item.coverPageIndex ?? 0
                            ),
                            contentMode: .fill,
                            minHeight: 0,
                            maxPixel: 540
                        )
                    }
                    .clipped()

                if isOpening {
                    Color.black.opacity(0.35)
                    ProgressView().tint(.white)
                }

                VStack {
                    HStack {
                        if item.isArchive {
                            Text("ZIP")
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.purple.opacity(0.85), in: Capsule())
                        }
                        Spacer()
                    }
                    Spacer()
                    HStack {
                        Spacer()
                        Text(item.itemCount > 1 ? "\(item.itemCount) P" : (item.isArchive ? "压缩包" : "\(item.itemCount) P"))
                            .font(.caption2.monospacedDigit().weight(.semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3)
                            .background(Color.black.opacity(0.65), in: Capsule())
                    }
                }
                .padding(7)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            VStack(alignment: .leading, spacing: 3) {
                Text(item.albumName)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)

                HStack {
                    Text(item.modelName)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Text(PMLFormatters.bytes(item.totalBytes))
                        .monospacedDigit()
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 2)
        }
        .padding(8)
        .pmlGlassCard(cornerRadius: 16)
    }
}

private struct GalleryCoverPickerSheet: View {
    let set: GallerySetV2
    let onSaved: (GallerySetV2) -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var loading = true
    @State private var saving = false
    @State private var errorText = ""
    @State private var pageCount = 0
    @State private var folderImages: [CreatorAssetV2] = []

    private let columns = [
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
    ]

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if !errorText.isEmpty {
                        Text(errorText).font(.caption).foregroundStyle(.red)
                    }
                    if loading {
                        ProgressView("加载候选封面中…")
                            .frame(maxWidth: .infinity)
                            .padding(.top, 40)
                    } else if set.isArchive, let aid = set.coverAssetId {
                        let count = max(1, min(pageCount, 18))
                        LazyVGrid(columns: columns, spacing: 10) {
                            ForEach(0..<count, id: \.self) { idx in
                                let isCurrent = (set.coverPageIndex ?? 0) == idx
                                Button {
                                    Task { await pickArchiveCover(assetId: aid, pageIndex: idx) }
                                } label: {
                                    ZStack(alignment: .bottomTrailing) {
                                        CachedImageView(
                                            url: model.apiClient?.comicPageURL(assetId: aid, pageIndex: idx, thumb: true),
                                            contentMode: .fill,
                                            minHeight: 130,
                                            maxPixel: 360
                                        )
                                        .aspectRatio(3.0 / 4.0, contentMode: .fit)
                                        .clipped()
                                        .clipShape(RoundedRectangle(cornerRadius: 8))
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 8)
                                                .stroke(isCurrent ? Color.purple : Color.clear, lineWidth: 2.5)
                                        )
                                        Text(isCurrent ? "当前" : "P\(idx + 1)")
                                            .font(.caption2.bold())
                                            .foregroundStyle(.white)
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(isCurrent ? Color.purple : Color.black.opacity(0.7), in: Capsule())
                                            .padding(5)
                                    }
                                }
                                .buttonStyle(.plain)
                                .disabled(saving)
                            }
                        }
                    } else {
                        LazyVGrid(columns: columns, spacing: 10) {
                            ForEach(Array(folderImages.prefix(24).enumerated()), id: \.element.id) { idx, img in
                                let isCurrent = set.coverAssetId == img.id
                                Button {
                                    Task { await pickFolderCover(assetId: img.id) }
                                } label: {
                                    ZStack(alignment: .bottomTrailing) {
                                        CachedImageView(
                                            url: model.apiClient?.v2CoverURL(assetId: img.id, isArchive: false),
                                            contentMode: .fill,
                                            minHeight: 130,
                                            maxPixel: 360
                                        )
                                        .aspectRatio(3.0 / 4.0, contentMode: .fit)
                                        .clipped()
                                        .clipShape(RoundedRectangle(cornerRadius: 8))
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 8)
                                                .stroke(isCurrent ? Color.purple : Color.clear, lineWidth: 2.5)
                                        )
                                        Text(isCurrent ? "当前" : "P\(idx + 1)")
                                            .font(.caption2.bold())
                                            .foregroundStyle(.white)
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(isCurrent ? Color.purple : Color.black.opacity(0.7), in: Capsule())
                                            .padding(5)
                                    }
                                }
                                .buttonStyle(.plain)
                                .disabled(saving)
                            }
                        }
                    }
                }
                .padding(14)
            }
            .navigationTitle("选择图集封面")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("关闭") { dismiss() }
                }
            }
            .task { await loadCandidates() }
        }
    }

    private func loadCandidates() async {
        guard let client = model.apiClient else { loading = false; return }
        defer { loading = false }
        if set.isArchive, let aid = set.coverAssetId {
            if let info = try? await client.inspectComicArchive(assetId: aid) {
                pageCount = info.pages.count
            } else {
                pageCount = Int(max(6, min(set.itemCount, 18)))
            }
        } else {
            let items = (try? await client.getGallerySetItems(setId: set.id)) ?? []
            folderImages = items.filter(\.isImage)
        }
    }

    private func pickArchiveCover(assetId: String, pageIndex: Int) async {
        guard let client = model.apiClient, !saving else { return }
        saving = true
        defer { saving = false }
        do {
            let updated = try await client.setGallerySetCover(setId: set.id, assetId: assetId, pageIndex: pageIndex)
            onSaved(updated)
            dismiss()
        } catch {
            errorText = "设置封面失败：\(error.localizedDescription)"
        }
    }

    private func pickFolderCover(assetId: String) async {
        guard let client = model.apiClient, !saving else { return }
        saving = true
        defer { saving = false }
        do {
            let updated = try await client.setGallerySetCover(setId: set.id, assetId: assetId, pageIndex: 0)
            onSaved(updated)
            dismiss()
        } catch {
            errorText = "设置封面失败：\(error.localizedDescription)"
        }
    }
}
