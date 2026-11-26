import SwiftUI

/// 漫画库 V1.5：主流漫画 App 体验（继续阅读横栏 + 420px 极速缩略图 + 极简书架 + 内页预览 + 选话宫格）。
struct ComicLibraryView: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var favs = EntityFavoritesStore.shared

    private enum ComicTab: String, CaseIterable, Identifiable {
        case shelf = "书架", categories = "分类", data = "数据"
        var id: String { rawValue }
    }

    @State private var tab: ComicTab = .shelf
    @State private var loading = true
    @State private var loadingMore = false
    @State private var items: [ComicWorkItem] = []
    @State private var continueReading: [ComicWorkItem] = []
    @State private var total: Int64 = 0
    @State private var hasMore = false
    @State private var stats: ComicLibraryStats?
    @State private var filterOptions: ComicFilterOptions?

    @State private var searchQuery = ""
    @State private var selectedCategory = ""
    @State private var selectedYear = ""
    @State private var selectedTag = ""
    @State private var selectedAuthor = ""
    @State private var selectedParody = ""
    @State private var selectedLanguage = "all"
    @State private var selectedReadStatus = "all"
    @State private var onlyUncensored = false
    @State private var onlyFullColor = false
    @State private var onlyDl = false
    @State private var onlyFavorites = false
    @State private var showFilterSheet = false
    @State private var sortMode: String? = nil

    private var activeFilterCount: Int {
        var n = 0
        if !selectedYear.isEmpty { n += 1 }
        if !selectedTag.isEmpty { n += 1 }
        if !selectedAuthor.isEmpty { n += 1 }
        if !selectedParody.isEmpty { n += 1 }
        if selectedLanguage != "all" { n += 1 }
        if selectedReadStatus != "all" { n += 1 }
        if onlyUncensored { n += 1 }
        if onlyFullColor { n += 1 }
        if onlyDl { n += 1 }
        if onlyFavorites { n += 1 }
        return n
    }

    @State private var selectedWork: ComicWorkItem?
    @State private var workDetail: ComicWorkDetail?
    @State private var volumeOrderDesc = false
    @State private var comicTarget: ComicArchiveTarget?
    @State private var folderTarget: ComicFolderTarget?

    @State private var busySync = false
    @State private var busyRebuild = false
    @State private var dataFeedback = ""
    @State private var batchProgress: ComicBatchProgress?
    @State private var batchPollTask: Task<Void, Never>?
    @State private var showSourceSheet = false
    @State private var sourceQuery = ""
    @State private var sourceLoading = false
    @State private var sourceCandidates: [ComicExternalCandidate] = []

    private let columns = [
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
    ]

    private let chapterColumns = [
        GridItem(.flexible(), spacing: 8),
        GridItem(.flexible(), spacing: 8),
        GridItem(.flexible(), spacing: 8),
    ]

    var body: some View {
        VStack(spacing: 0) {
            Picker("分区", selection: $tab) {
                ForEach(ComicTab.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal)
            .padding(.vertical, 8)
            .onChange(of: tab) { _, newTab in
                Task {
                    if newTab == .shelf {
                        await loadShelf(reset: true)
                    } else if newTab == .categories {
                        await loadFilterOptions()
                    } else {
                        await loadStats()
                    }
                }
            }

            switch tab {
            case .shelf:
                shelfView
            case .categories:
                categoriesView
            case .data:
                dataView
            }
        }
        .navigationTitle("漫画库 (\(stats?.totalWorks ?? total))")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchQuery, prompt: "搜索书名、作者、社团、原作 IP")
        .task(id: searchQuery) {
            try? await Task.sleep(for: .milliseconds(320))
            guard !Task.isCancelled else { return }
            if tab == .shelf { await loadShelf(reset: true) }
        }
        .task {
            await favs.ensureLoaded(client: model.apiClient)
            await loadStats()
            await loadFilterOptions()
            await loadContinueReading()
            await loadShelf(reset: true)
            await pollComicBatchOnce()
        }
        .onDisappear { batchPollTask?.cancel() }
        .sheet(item: $selectedWork) { work in
            comicDetailSheet(work)
        }
        .sheet(isPresented: $showFilterSheet) {
            filterSheetView
        }
        .fullScreenCover(item: $comicTarget) { target in
            ComicReaderView(target: target).environmentObject(model)
        }
        .fullScreenCover(item: $folderTarget) { target in
            ComicFolderReaderView(target: target).environmentObject(model)
        }
    }

    private func coverURL(assetId: String?, isArchive: Bool, pageIndex: Int = 0) -> URL? {
        guard let aid = assetId, !aid.isEmpty, let client = model.apiClient else { return nil }
        return isArchive
            ? client.comicPageURL(assetId: aid, pageIndex: pageIndex, thumb: true)
            : client.v2CoverURL(assetId: aid, isArchive: false)
    }

    private var shelfView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                // 继续阅读横栏
                if tab == .shelf && searchQuery.isEmpty && selectedAuthor.isEmpty && selectedParody.isEmpty && !continueReading.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("继续阅读", systemImage: "clock.arrow.circlepath")
                            .font(.subheadline.bold())
                            .padding(.horizontal, 14)
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 10) {
                                ForEach(continueReading) { cw in
                                    Button {
                                        openDetail(cw)
                                    } label: {
                                        VStack(alignment: .leading, spacing: 4) {
                                            ZStack(alignment: .bottom) {
                                                CachedImageView(
                                                    url: coverURL(assetId: cw.coverAssetId, isArchive: cw.coverIsArchive, pageIndex: cw.coverPageIndex ?? 0),
                                                    contentMode: .fill,
                                                    minHeight: 130,
                                                    maxPixel: 420
                                                )
                                                .frame(width: 96, height: 142)
                                                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                                                GeometryReader { geo in
                                                    ZStack(alignment: .leading) {
                                                        Color.black.opacity(0.4)
                                                        (cw.readStatus == "finished" ? Color.green : Color.purple)
                                                            .frame(width: geo.size.width * CGFloat(min(100, max(5, cw.readPercent ?? 10))) / 100.0)
                                                    }
                                                }
                                                .frame(height: 3)
                                                .clipShape(RoundedRectangle(cornerRadius: 2))
                                            }
                                            Text(cw.title)
                                                .font(.caption2.weight(.semibold))
                                                .foregroundStyle(.primary)
                                                .lineLimit(1)
                                            Text(cw.lastReadVolumeLabel ?? "继续阅读")
                                                .font(.system(size: 10))
                                                .foregroundStyle(.purple)
                                                .lineLimit(1)
                                        }
                                        .frame(width: 96)
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                            .padding(.horizontal, 14)
                        }
                    }
                }

                // 快捷分类频道条
                if let cats = filterOptions?.categories, !cats.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            filterPill("全部", active: selectedCategory.isEmpty) {
                                selectedCategory = ""
                                Task { await loadShelf(reset: true) }
                            }
                            ForEach(cats) { c in
                                filterPill("\(c.name) \(c.count)", active: selectedCategory == c.name) {
                                    selectedCategory = (selectedCategory == c.name ? "" : c.name)
                                    Task { await loadShelf(reset: true) }
                                }
                            }
                        }
                        .padding(.horizontal, 14)
                    }
                }

                // 精简工具条：筛选面板入口 + 最新出版 + 随机漫步
                HStack(spacing: 8) {
                    Button {
                        showFilterSheet = true
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "line.3.horizontal.decrease.circle")
                            Text(activeFilterCount > 0 ? "筛选 (\(activeFilterCount))" : "筛选")
                        }
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(activeFilterCount > 0 ? Color.purple : Color.secondary.opacity(0.14), in: Capsule())
                        .foregroundStyle(activeFilterCount > 0 ? .white : .primary)
                    }
                    filterPill("最新出版", active: sortMode == "published_desc") {
                        sortMode = (sortMode == "published_desc" ? nil : "published_desc")
                        Task { await loadShelf(reset: true) }
                    }
                    Button {
                        sortMode = "random"
                        Task { await loadShelf(reset: true) }
                    } label: {
                        Label("随机", systemImage: "shuffle")
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(sortMode == "random" ? Color.purple : Color.secondary.opacity(0.14), in: Capsule())
                            .foregroundStyle(sortMode == "random" ? .white : .primary)
                    }
                    Spacer()
                }
                .padding(.horizontal, 14)

                if !selectedAuthor.isEmpty || !selectedParody.isEmpty || !selectedTag.isEmpty || !selectedYear.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            if !selectedTag.isEmpty {
                                Button { selectedTag = ""; Task { await loadShelf(reset: true) } } label: {
                                    Label("#\(selectedTag)", systemImage: "xmark.circle.fill").font(.caption.bold())
                                }.buttonStyle(.borderedProminent).controlSize(.mini).tint(.purple)
                            }
                            if !selectedYear.isEmpty {
                                Button { selectedYear = ""; Task { await loadShelf(reset: true) } } label: {
                                    Label("\(selectedYear)年", systemImage: "xmark.circle.fill").font(.caption.bold())
                                }.buttonStyle(.borderedProminent).controlSize(.mini).tint(.indigo)
                            }
                            if !selectedAuthor.isEmpty {
                                Button { selectedAuthor = ""; Task { await loadShelf(reset: true) } } label: {
                                    Label("作者：\(selectedAuthor)", systemImage: "xmark.circle.fill").font(.caption.bold())
                                }.buttonStyle(.borderedProminent).controlSize(.mini).tint(.purple)
                            }
                            if !selectedParody.isEmpty {
                                Button { selectedParody = ""; Task { await loadShelf(reset: true) } } label: {
                                    Label("原作：\(selectedParody)", systemImage: "xmark.circle.fill").font(.caption.bold())
                                }.buttonStyle(.borderedProminent).controlSize(.mini).tint(.blue)
                            }
                        }
                        .padding(.horizontal, 14)
                    }
                }

                if loading && items.isEmpty {
                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(0..<12, id: \.self) { _ in
                            VStack(alignment: .leading, spacing: 6) {
                                RoundedRectangle(cornerRadius: 8)
                                    .fill(Color.secondary.opacity(0.16))
                                    .aspectRatio(2 / 3, contentMode: .fit)
                                RoundedRectangle(cornerRadius: 4)
                                    .fill(Color.secondary.opacity(0.14))
                                    .frame(height: 11)
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                } else if items.isEmpty {
                    ContentUnavailableView("暂无匹配漫画", systemImage: "book.closed", description: Text("切换到「数据」页可同步并自动聚类漫画库。"))
                        .padding(.top, 40)
                } else {
                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(items) { work in
                            Button {
                                openDetail(work)
                            } label: {
                                VStack(alignment: .leading, spacing: 5) {
                                    ZStack(alignment: .topTrailing) {
                                        CachedImageView(
                                            url: coverURL(assetId: work.coverAssetId, isArchive: work.coverIsArchive, pageIndex: work.coverPageIndex ?? 0),
                                            contentMode: .fill,
                                            minHeight: 150,
                                            maxPixel: 420
                                        )
                                        .aspectRatio(2 / 3, contentMode: .fit)
                                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                                        if work.volumeCount > 1 {
                                            Text("\(work.volumeCount)卷")
                                                .font(.system(size: 9, weight: .bold))
                                                .padding(.horizontal, 5)
                                                .padding(.vertical, 2)
                                                .background(Color.black.opacity(0.76), in: RoundedRectangle(cornerRadius: 4))
                                                .foregroundStyle(.white)
                                                .padding(5)
                                        } else if work.language != "zh" {
                                            Text("生")
                                                .font(.system(size: 9, weight: .bold))
                                                .padding(.horizontal, 5)
                                                .padding(.vertical, 2)
                                                .background(Color.gray.opacity(0.82), in: RoundedRectangle(cornerRadius: 4))
                                                .foregroundStyle(.white)
                                                .padding(5)
                                        }

                                        if let st = work.readStatus, st != "unread" {
                                            VStack {
                                                Spacer()
                                                GeometryReader { geo in
                                                    ZStack(alignment: .leading) {
                                                        Color.black.opacity(0.45)
                                                        (st == "finished" ? Color.green : Color.purple)
                                                            .frame(width: geo.size.width * CGFloat(min(100, max(5, work.readPercent ?? 10))) / 100.0)
                                                    }
                                                }
                                                .frame(height: 3)
                                            }
                                            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                                        }
                                    }

                                    Text(work.title)
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(.primary)
                                        .lineLimit(2)
                                        .multilineTextAlignment(.leading)

                                    Text((work.authors.first ?? work.circle ?? "未知作者") + (work.publishedAt.map { " · \($0.prefix(4))" } ?? ""))
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                            }
                            .buttonStyle(.plain)
                            .onAppear {
                                if work.id == items.last?.id && hasMore && !loadingMore {
                                    Task { await loadShelf(reset: false) }
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                }

                if loadingMore {
                    ProgressView().frame(maxWidth: .infinity).padding()
                }
            }
            .padding(.vertical, 8)
        }
        .refreshable {
            await loadStats()
            await loadContinueReading()
            await loadShelf(reset: true)
        }
    }

    private var categoriesView: some View {
        List {
            if let cats = filterOptions?.categories, !cats.isEmpty {
                Section("漫画形态") {
                    ForEach(cats) { c in
                        Button {
                            selectedCategory = c.name
                            tab = .shelf
                        } label: {
                            HStack {
                                Label(c.name, systemImage: "folder.fill")
                                    .foregroundStyle(.purple)
                                Spacer()
                                Text("\(c.count) 部").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }

            if let genres = filterOptions?.genres, !genres.isEmpty {
                Section("阅读题材") {
                    ForEach(genres.prefix(30)) { t in
                        Button {
                            selectedTag = t.name
                            tab = .shelf
                        } label: {
                            HStack {
                                Text("#\(t.name)").foregroundStyle(.primary)
                                Spacer()
                                Text("\(t.count)").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }

            if let years = filterOptions?.years, !years.isEmpty {
                Section("出版年份") {
                    ForEach(years) { y in
                        Button {
                            selectedYear = y.name
                            tab = .shelf
                        } label: {
                            HStack {
                                Text("\(y.name) 年").foregroundStyle(.primary)
                                Spacer()
                                Text("\(y.count) 部").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }

            if let parodies = filterOptions?.parodies, !parodies.isEmpty {
                Section("热门原作 IP") {
                    ForEach(parodies.prefix(25)) { p in
                        Button {
                            selectedParody = p.name
                            tab = .shelf
                        } label: {
                            HStack {
                                Text(p.name).foregroundStyle(.primary)
                                Spacer()
                                Text("\(p.count) 部").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
    }

    private var dataView: some View {
        List {
            Section("漫画库总览") {
                HStack(spacing: 0) {
                    statCell("\(stats?.totalWorks ?? 0)", "作品", .primary)
                    statCell("\(stats?.totalSeries ?? 0)", "系列", .purple)
                    statCell("\(stats?.enrichedCount ?? 0)", "已补全", .green)
                    statCell("\(stats?.totalVolumes ?? 0)", "卷/话", .blue)
                    statCell("\(stats?.totalAuthors ?? 0)", "作者", .orange)
                }
                .padding(.vertical, 4)

                Button {
                    Task { await syncAndRebuild() }
                } label: {
                    HStack {
                        Spacer()
                        if busySync {
                            ProgressView().controlSize(.small).padding(.trailing, 4)
                            Text("增量同步并建库中…")
                        } else {
                            Label("增量同步并建库", systemImage: "bolt.fill")
                        }
                        Spacer()
                    }
                    .font(.subheadline.weight(.semibold))
                }
                .buttonStyle(.borderedProminent)
                .tint(.purple)
                .disabled(busySync || busyRebuild)

                Button {
                    Task { await localRebuild() }
                } label: {
                    Label(busyRebuild ? "重新聚类中…" : "本地快速重新聚类", systemImage: "arrow.triangle.2.circlepath")
                }
                .disabled(busySync || busyRebuild)

                if let bp = batchProgress, bp.running {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("正在补全：\(bp.currentTitle ?? "准备中…") (\(bp.completed)/\(bp.total))")
                            .font(.caption2)
                        ProgressView(value: Double(bp.completed), total: Double(max(bp.total, 1)))
                        Text("成功 \(bp.succeeded) · 未命中 \(bp.failed)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    Button("停止外网补全", role: .destructive) {
                        Task {
                            batchProgress = try? await model.apiClient?.stopComicBatchScrape()
                        }
                    }
                } else {
                    let pendingEnrich = max(0, (stats?.totalWorks ?? 0) - (stats?.enrichedCount ?? 0))
                    Button {
                        Task { await startBatchScrape() }
                    } label: {
                        Label("外网补全标签与简介（\(pendingEnrich)）", systemImage: "sparkles")
                    }
                    .tint(.purple)
                    .disabled(pendingEnrich == 0)
                }

                NavigationLink {
                    LibraryModulesSettingsView()
                } label: {
                    Label("管理漫画目录（支持多路径）", systemImage: "folder.badge.gearshape")
                }

                if !dataFeedback.isEmpty {
                    Text(dataFeedback).font(.caption).foregroundStyle(.purple)
                }
            }
        }
        .listStyle(.insetGrouped)
    }

    private func comicDetailSheet(_ work: ComicWorkItem) -> some View {
        let vols = workDetail?.volumes ?? []
        let orderedVols = volumeOrderDesc ? Array(vols.reversed()) : vols
        let resumeVol: ComicVolumeItem? = {
            if let lastId = workDetail?.work.lastReadVolumeId, let hit = vols.first(where: { $0.id == lastId }) {
                return hit
            }
            return vols.first
        }()

        return NavigationStack {
            List {
                Section {
                    HStack(alignment: .top, spacing: 14) {
                        CachedImageView(
                            url: coverURL(
                                assetId: workDetail?.work.coverAssetId ?? work.coverAssetId,
                                isArchive: workDetail?.work.coverIsArchive ?? work.coverIsArchive,
                                pageIndex: workDetail?.work.coverPageIndex ?? work.coverPageIndex ?? 0
                            ),
                            contentMode: .fill,
                            minHeight: 150,
                            maxPixel: 420
                        )
                        .frame(width: 105, height: 155)
                        .clipShape(RoundedRectangle(cornerRadius: 8))

                        VStack(alignment: .leading, spacing: 6) {
                            Text(work.title).font(.headline)
                            if !work.authors.isEmpty {
                                Text("作者：\(work.authors.joined(separator: " / "))")
                                    .font(.caption)
                                    .foregroundStyle(.purple)
                                    .onTapGesture {
                                        selectedAuthor = work.authors[0]
                                        selectedWork = nil
                                        tab = .shelf
                                    }
                            }
                            if let c = work.circle, !c.isEmpty {
                                Text("社团：\(c)").font(.caption2).foregroundStyle(.secondary)
                            }
                            if let p = work.parody, !p.isEmpty {
                                Text("原作：\(p)").font(.caption2).foregroundStyle(.blue)
                                    .onTapGesture {
                                        selectedParody = p
                                        selectedWork = nil
                                        tab = .shelf
                                    }
                            }
                            if !work.translators.isEmpty {
                                Text("汉化：\(work.translators.joined(separator: " · "))")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                            if let pub = workDetail?.work.publishedAt ?? work.publishedAt, !pub.isEmpty {
                                Text("出版：\(pub)\(work.category.map { " · \($0)" } ?? "")")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                            if let rating = workDetail?.work.rating ?? work.rating {
                                Text("★ \(String(format: "%.1f", rating)) · \((workDetail?.work.source ?? work.source) == "ehentai" ? "E-Hentai" : "Bangumi")")
                                    .font(.caption2.bold())
                                    .foregroundStyle(.orange)
                            }
                            HStack(spacing: 8) {
                                if let targetVol = resumeVol {
                                    Button {
                                        openVolume(targetVol, workId: work.id)
                                    } label: {
                                        Label(workDetail?.work.lastReadVolumeLabel != nil ? "继续阅读" : "开始阅读", systemImage: "book.fill")
                                    }
                                    .buttonStyle(.borderedProminent)
                                    .controlSize(.small)
                                    .tint(.purple)
                                }
                                Button {
                                    sourceQuery = work.title
                                    sourceCandidates = []
                                    showSourceSheet = true
                                    Task { await runManualSourceSearch(workId: work.id, useCoverHash: true) }
                                } label: {
                                    Label("搜源", systemImage: "globe")
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                                Button {
                                    Task { await favs.toggle(.comic, id: work.id, client: model.apiClient) }
                                } label: {
                                    Image(systemName: favs.isComicFav(work.id) ? "heart.fill" : "heart")
                                        .foregroundStyle(.pink)
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                            }
                            .padding(.top, 4)
                        }
                    }
                    .padding(.vertical, 4)
                    if let summary = workDetail?.work.summary ?? work.summary, !summary.isEmpty {
                        Text(summary)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                // 内页画风预览 & 一键切换封面
                if let firstVol = vols.first, firstVol.isArchive {
                    let curCoverIdx = workDetail?.work.coverPageIndex ?? work.coverPageIndex ?? 0
                    Section("内页预览（可一键更换封面）") {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 10) {
                                ForEach(0..<6, id: \.self) { pIdx in
                                    VStack(spacing: 4) {
                                        Button {
                                            openVolume(firstVol, workId: work.id)
                                        } label: {
                                            CachedImageView(
                                                url: coverURL(assetId: firstVol.assetId, isArchive: true, pageIndex: pIdx),
                                                contentMode: .fill,
                                                minHeight: 110,
                                                maxPixel: 360
                                            )
                                            .frame(width: 78, height: 116)
                                            .clipShape(RoundedRectangle(cornerRadius: 6))
                                            .overlay(
                                                RoundedRectangle(cornerRadius: 6)
                                                    .stroke(curCoverIdx == pIdx ? Color.purple : Color.clear, lineWidth: 2)
                                            )
                                        }
                                        .buttonStyle(.plain)

                                        Button {
                                            Task { await setCoverPage(workId: work.id, assetId: firstVol.assetId, pageIndex: pIdx) }
                                        } label: {
                                            Text(curCoverIdx == pIdx ? "当前封面" : "设为封面")
                                                .font(.system(size: 10, weight: curCoverIdx == pIdx ? .bold : .medium))
                                                .padding(.horizontal, 6)
                                                .padding(.vertical, 3)
                                                .background(curCoverIdx == pIdx ? Color.purple : Color.secondary.opacity(0.16), in: Capsule())
                                                .foregroundStyle(curCoverIdx == pIdx ? .white : .primary)
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                            }
                            .padding(.vertical, 4)
                        }
                    }
                }

                // 多卷/多话：选话宫格 + 正/倒序切换
                if vols.count > 1 {
                    Section {
                        LazyVGrid(columns: chapterColumns, spacing: 8) {
                            ForEach(orderedVols) { vol in
                                let isCurrent = workDetail?.work.lastReadVolumeId == vol.id
                                Button {
                                    openVolume(vol, workId: work.id)
                                } label: {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(vol.volumeLabel)
                                            .font(.caption.bold())
                                            .lineLimit(1)
                                        Text(PMLFormatters.bytes(vol.sizeBytes))
                                            .font(.system(size: 10))
                                            .foregroundStyle(.secondary)
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(8)
                                    .background(
                                        isCurrent ? Color.purple.opacity(0.2) : Color.secondary.opacity(0.1),
                                        in: RoundedRectangle(cornerRadius: 8)
                                    )
                                    .overlay(
                                        RoundedRectangle(cornerRadius: 8)
                                            .stroke(isCurrent ? Color.purple : Color.clear, lineWidth: 1)
                                    )
                                    .opacity(vol.completed == true ? 0.55 : 1.0)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                        .padding(.vertical, 4)
                    } header: {
                        HStack {
                            Text("选话目录（\(vols.count)）")
                            Spacer()
                            Button(volumeOrderDesc ? "倒序" : "正序") {
                                volumeOrderDesc.toggle()
                            }
                            .font(.caption)
                        }
                    }
                }
            }
            .navigationTitle("漫画详情")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("关闭") { selectedWork = nil }
                }
            }
            .sheet(isPresented: $showSourceSheet) {
                NavigationStack {
                    List {
                        Section {
                            HStack {
                                TextField("书名或作者关键词", text: $sourceQuery)
                                    .textFieldStyle(.roundedBorder)
                                Button("搜索") {
                                    Task { await runManualSourceSearch(workId: work.id, useCoverHash: false) }
                                }
                                .disabled(sourceLoading)
                            }
                        }
                        if sourceLoading {
                            ProgressView("搜索 E-Hentai 与 Bangumi 中…")
                        } else if sourceCandidates.isEmpty {
                            Text("暂无匹配结果").font(.caption).foregroundStyle(.secondary)
                        } else {
                            ForEach(sourceCandidates) { cand in
                                VStack(alignment: .leading, spacing: 6) {
                                    HStack {
                                        Text(cand.source == "ehentai" ? "E-Hentai" : "Bangumi")
                                            .font(.caption2.bold())
                                            .foregroundStyle(.blue)
                                        Text(cand.matchReason)
                                            .font(.caption2)
                                            .foregroundStyle(.purple)
                                        Spacer()
                                        Button("采用") {
                                            Task {
                                                if let updated = try? await model.apiClient?.applyComicExternalSource(workId: work.id, candidate: cand) {
                                                    workDetail = updated
                                                    showSourceSheet = false
                                                    await loadStats()
                                                    await loadShelf(reset: true)
                                                }
                                            }
                                        }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.mini)
                                        .tint(.purple)
                                    }
                                    Text(cand.title).font(.caption.bold())
                                    if !cand.tags.isEmpty {
                                        Text(cand.tags.prefix(8).joined(separator: " · "))
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                .padding(.vertical, 4)
                            }
                        }
                    }
                    .navigationTitle("手动搜源")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .topBarTrailing) {
                            Button("关闭") { showSourceSheet = false }
                        }
                    }
                }
            }
        }
    }

    private func filterPill(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(active ? Color.purple : Color.secondary.opacity(0.14), in: Capsule())
            .foregroundStyle(active ? .white : .primary)
    }

    private func statCell(_ val: String, _ label: String, _ color: Color) -> some View {
        VStack(spacing: 2) {
            Text(val).font(.headline.monospacedDigit()).foregroundStyle(color)
            Text(label).font(.caption2).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
    }

    private func openDetail(_ work: ComicWorkItem) {
        selectedWork = work
        workDetail = nil
        volumeOrderDesc = false
        Task {
            guard let client = model.apiClient else { return }
            workDetail = try? await client.getComicWorkDetail(workId: work.id)
        }
    }

    private func setCoverPage(workId: String, assetId: String, pageIndex: Int) async {
        guard let client = model.apiClient else { return }
        if let updated = try? await client.setComicWorkCover(workId: workId, pageIndex: pageIndex, assetId: assetId) {
            if let d = workDetail, d.work.id == workId {
                workDetail = ComicWorkDetail(
                    work: updated,
                    volumes: d.volumes,
                    relatedByAuthor: d.relatedByAuthor,
                    relatedByParody: d.relatedByParody
                )
            }
            if let idx = items.firstIndex(where: { $0.id == workId }) {
                items[idx] = updated
            }
            if let idx = continueReading.firstIndex(where: { $0.id == workId }) {
                continueReading[idx] = updated
            }
        }
    }

    private func openVolume(_ vol: ComicVolumeItem, workId: String) {
        selectedWork = nil
        Task {
            if let client = model.apiClient {
                try? await client.saveComicReadingProgress(
                    workId: workId,
                    volumeId: vol.id,
                    assetId: vol.assetId,
                    currentPage: max(vol.readPage ?? 0, 1),
                    totalPages: vol.pageCount
                )
                await loadContinueReading()
            }
            try? await Task.sleep(nanoseconds: 320_000_000)
            if vol.isArchive {
                comicTarget = ComicArchiveTarget(id: vol.assetId, title: vol.fileName)
            } else if let folder = vol.folderPath {
                let roots = (try? await model.apiClient?.libraryRoots()) ?? []
                let root = roots.first(where: { $0.kind == "comics" })
                    ?? LibraryRoot(id: "comics", kind: "comics", remotePath: folder, createdAt: 0)
                folderTarget = ComicFolderTarget(root: root, path: folder, startPath: nil)
            }
        }
    }

    private func loadStats() async {
        guard let client = model.apiClient else { return }
        stats = try? await client.getComicLibraryStats()
    }

    private func loadContinueReading() async {
        guard let client = model.apiClient else { return }
        continueReading = (try? await client.listContinueReadingComics(limit: 12)) ?? []
    }

    private func loadFilterOptions() async {
        guard let client = model.apiClient else { return }
        filterOptions = try? await client.listComicFilterOptions()
    }

    private var filterSheetView: some View {
        NavigationStack {
            Form {
                Section("阅读状态") {
                    Picker("状态", selection: $selectedReadStatus) {
                        Text("全部").tag("all")
                        Text("未读").tag("unread")
                        Text("在读").tag("reading")
                        Text("已读").tag("finished")
                    }
                    .pickerStyle(.segmented)
                }
                Section("语言与属性") {
                    Picker("语言", selection: $selectedLanguage) {
                        Text("全部").tag("all")
                        Text("中文").tag("zh")
                        Text("生肉").tag("ja")
                    }
                    .pickerStyle(.segmented)
                    Toggle("无修正", isOn: $onlyUncensored)
                    Toggle("全彩", isOn: $onlyFullColor)
                    Toggle("DL版", isOn: $onlyDl)
                    Toggle("仅看收藏", isOn: $onlyFavorites)
                }
                if let years = filterOptions?.years, !years.isEmpty {
                    Section("出版年份") {
                        Picker("年份", selection: $selectedYear) {
                            Text("全部年份").tag("")
                            ForEach(years) { y in
                                Text("\(y.name) 年 (\(y.count))").tag(y.name)
                            }
                        }
                    }
                }
                if let genres = filterOptions?.genres, !genres.isEmpty {
                    Section("题材标签") {
                        Picker("标签", selection: $selectedTag) {
                            Text("全部题材").tag("")
                            ForEach(genres.prefix(30)) { g in
                                Text("#\(g.name) (\(g.count))").tag(g.name)
                            }
                        }
                    }
                }
                if let parodies = filterOptions?.parodies, !parodies.isEmpty {
                    Section("原作 IP") {
                        Picker("原作", selection: $selectedParody) {
                            Text("全部原作").tag("")
                            ForEach(parodies.prefix(25)) { p in
                                Text("\(p.name) (\(p.count))").tag(p.name)
                            }
                        }
                    }
                }
                Section {
                    Button("清空全部筛选", role: .destructive) {
                        selectedCategory = ""
                        selectedYear = ""
                        selectedTag = ""
                        selectedAuthor = ""
                        selectedParody = ""
                        selectedLanguage = "all"
                        selectedReadStatus = "all"
                        onlyUncensored = false
                        onlyFullColor = false
                        onlyDl = false
                        onlyFavorites = false
                        sortMode = nil
                    }
                }
            }
            .navigationTitle("筛选漫画")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("应用") {
                        showFilterSheet = false
                        Task { await loadShelf(reset: true) }
                    }
                }
            }
        }
    }

    private func loadShelf(reset: Bool) async {
        guard let client = model.apiClient else { return }
        if reset { loading = true } else { loadingMore = true }
        defer { loading = false; loadingMore = false }
        if let res = try? await client.queryComicLibrary(
            query: searchQuery.trimmingCharacters(in: .whitespacesAndNewlines),
            category: selectedCategory,
            year: selectedYear,
            author: selectedAuthor,
            parody: selectedParody,
            tag: selectedTag,
            language: selectedLanguage,
            readStatus: selectedReadStatus,
            onlyUncensored: onlyUncensored,
            onlyFullColor: onlyFullColor,
            onlyDl: onlyDl,
            onlyFavorites: onlyFavorites,
            sort: sortMode,
            limit: 48,
            offset: reset ? 0 : items.count
        ) {
            items = reset ? res.items : (items + res.items)
            total = res.total
            hasMore = res.hasMore
        }
    }

    private func syncAndRebuild() async {
        guard let client = model.apiClient else { return }
        busySync = true
        dataFeedback = "正在增量同步并自动聚类建库…"
        defer { busySync = false }
        do {
            _ = try await client.scanLibraryModule(moduleId: "comics", full: false)
            let st = try await client.rebuildComicLibrary()
            stats = st
            dataFeedback = "完成：\(st.totalWorks) 部作品（\(st.totalSeries) 系列），\(st.totalVolumes) 卷/话，\(st.totalAuthors) 位作者。"
            await loadShelf(reset: true)
        } catch {
            dataFeedback = "同步失败：\(error.localizedDescription)"
        }
    }

    private func localRebuild() async {
        guard let client = model.apiClient else { return }
        busyRebuild = true
        dataFeedback = "正在本地重新解析聚类…"
        defer { busyRebuild = false }
        do {
            let st = try await client.rebuildComicLibrary()
            stats = st
            dataFeedback = "重建完成：\(st.totalWorks) 部作品（\(st.totalSeries) 系列），\(st.totalVolumes) 卷/话，\(st.totalAuthors) 位作者。"
            await loadShelf(reset: true)
        } catch {
            dataFeedback = "重建失败：\(error.localizedDescription)"
        }
    }

    private func pollComicBatchOnce() async {
        guard let client = model.apiClient else { return }
        batchProgress = try? await client.getComicBatchScrapeProgress()
        if batchProgress?.running == true {
            beginComicBatchPolling()
        }
    }

    private func startBatchScrape() async {
        guard let client = model.apiClient else { return }
        batchProgress = try? await client.startComicBatchScrape(useCoverHash: true)
        beginComicBatchPolling()
    }

    private func beginComicBatchPolling() {
        batchPollTask?.cancel()
        batchPollTask = Task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                guard let client = model.apiClient else { break }
                let prev = batchProgress?.completed ?? -1
                batchProgress = try? await client.getComicBatchScrapeProgress()
                if (batchProgress?.completed ?? -1) != prev {
                    await loadStats()
                }
                if batchProgress?.running != true {
                    await loadStats()
                    await loadShelf(reset: true)
                    break
                }
            }
        }
    }

    private func runManualSourceSearch(workId: String, useCoverHash: Bool) async {
        guard let client = model.apiClient else { return }
        sourceLoading = true
        defer { sourceLoading = false }
        sourceCandidates = (try? await client.searchComicOnlineSources(
            workId: workId,
            query: sourceQuery.trimmingCharacters(in: .whitespacesAndNewlines),
            useCoverHash: useCoverHash
        )) ?? []
    }
}
