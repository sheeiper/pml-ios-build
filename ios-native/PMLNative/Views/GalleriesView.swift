import SwiftUI

private enum GallerySortV2: String, CaseIterable, Identifiable {
    case latest = "最新入库"
    case count = "页数最多"
    case size = "体积最大"
    case name = "名称 A-Z"

    var id: String { rawValue }
    var apiValue: String {
        switch self {
        case .latest: return "latest"
        case .count: return "count"
        case .size: return "size"
        case .name: return "name"
        }
    }
}

// MARK: - 写真图集大厅 V2

/// 写真图集 V2：15 位模特横向筛选 + 3:4 竖版图集卡片 + 文件夹图集（ImageViewer）/ 压缩包图集（ComicReaderView）。
struct GalleriesView: View {
    @EnvironmentObject private var model: AppModel

    @State private var models: [GalleryModelStat] = []
    @State private var selectedModel: String? = nil
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
    @State private var imageViewerContext: ImageViewerContext?
    @State private var comicTarget: ComicArchiveTarget?

    private let pageSize = 60
    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12),
    ]

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
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("加载写真图集…").foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 44)
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
                    Image(systemName: (selectedModel != nil || sortMode != .latest) ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                }
                .accessibilityLabel("图集筛选与排序")
            }
        }
        .sheet(isPresented: $showFilterSheet) {
            NavigationStack {
                Form {
                    Section("排序方式") {
                        Picker("排序", selection: $sortMode) {
                            ForEach(GallerySortV2.allCases) { mode in
                                Text(mode.rawValue).tag(mode)
                            }
                        }
                        .pickerStyle(.segmented)
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
                    }
                }
                .navigationTitle("图集筛选")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) { Button("完成") { showFilterSheet = false } }
                }
            }
            .presentationDetents([.medium, .large])
        }
        .fullScreenCover(item: $imageViewerContext) { ctx in
            ImageViewer(context: ctx)
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target, kind: .gallery)
        }
        .task { await loadAll() }
        .onChange(of: selectedModel) { _, _ in
            Task { await loadSetsOnly() }
        }
        .onChange(of: sortMode) { _, _ in
            Task { await loadSetsOnly() }
        }
        .onChange(of: searchText) { _, _ in
            Task { await loadSetsOnly() }
        }
    }

    private var modelPillBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                V2FilterPill(
                    title: "全部模特",
                    count: models.isEmpty ? nil : models.reduce(0) { $0 + $1.setCount },
                    isSelected: selectedModel == nil
                ) {
                    selectedModel = nil
                }

                ForEach(models) { item in
                    V2FilterPill(
                        title: item.modelName,
                        count: item.setCount,
                        isSelected: selectedModel == item.modelName
                    ) {
                        selectedModel = item.modelName
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
            async let modelReq = client.listGalleryModels()
            async let setsReq = client.listGallerySets(
                modelName: selectedModel,
                query: searchText.trimmingCharacters(in: .whitespacesAndNewlines),
                sort: sortMode.apiValue,
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
        syncing = true
        noticeText = ""
        errorText = ""
        defer { syncing = false }
        do {
            let result = try await client.scanLibraryModule(moduleId: "galleries", full: false)
            noticeText = "图集同步完成：扫描 \(result.scan.scanned) 项，共索引 \(result.rebuild.entityCount) 套图集"
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
                imageViewerContext = ImageViewerContext(
                    entries: images.map { $0.toRemoteBrowserEntry() },
                    startIndex: 0
                )
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
                            url: client?.v2CoverURL(assetId: item.coverAssetId, isArchive: item.isArchive),
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
