import SwiftUI

/// 漫画库 V1.5：主流漫画 App 体验（继续阅读横栏 + 420px 极速缩略图 + 极简书架 + 内页预览 + 选话宫格）。
struct ComicLibraryView: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var favs = EntityFavoritesStore.shared

    private enum ComicTab: String, CaseIterable, Identifiable {
        case shelf = "书架", galleries = "图集", online = "在线", data = "数据"
        var id: String { rawValue }
    }

    init(initialTab: String? = nil) {
        if let initialTab, let matched = ComicTab(rawValue: initialTab) {
            _tab = State(initialValue: matched)
        } else {
            _tab = State(initialValue: .shelf)
        }
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
    @State private var showOnlineFilterSheet = false
    @State private var sortMode: String? = nil

    private var activeFilterCount: Int {
        var n = 0
        if !selectedCategory.isEmpty { n += 1 }
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
        if sortMode != nil { n += 1 }
        return n
    }

    @State private var selectedWork: ComicWorkItem?
    @State private var workDetail: ComicWorkDetail?
    @State private var coverPickerWork: ComicWorkItem?
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
                    } else if newTab == .data {
                        await loadStats()
                    }
                }
            }

            switch tab {
            case .shelf:
                shelfView
                    .searchable(text: $searchQuery, prompt: "搜索书名、作者、社团、原作 IP")
            case .galleries:
                GalleriesView()
            case .online:
                OnlineComicsPaneView(showFilterSheet: $showOnlineFilterSheet)
            case .data:
                dataView
            }
        }
        .navigationTitle("漫画库 (\(stats?.totalWorks ?? total))")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if tab == .shelf {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showFilterSheet = true
                    } label: {
                        Image(systemName: activeFilterCount > 0 ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                    }
                    .accessibilityLabel("筛选与排序")
                }
            } else if tab == .online {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        showOnlineFilterSheet = true
                    } label: {
                        Image(systemName: "line.3.horizontal.decrease.circle")
                    }
                    .accessibilityLabel("在线漫画源与筛选")
                }
            }
        }
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
        .sheet(item: $coverPickerWork) { work in
            ComicCoverPickerSheet(work: work) { updated in
                if let idx = items.firstIndex(where: { $0.id == updated.id }) {
                    items[idx] = updated
                }
                if let idx = continueReading.firstIndex(where: { $0.id == updated.id }) {
                    continueReading[idx] = updated
                }
                if let d = workDetail, d.work.id == updated.id {
                    workDetail = ComicWorkDetail(
                        work: updated,
                        volumes: d.volumes,
                        relatedByAuthor: d.relatedByAuthor,
                        relatedByParody: d.relatedByParody
                    )
                }
            }
            .environmentObject(model)
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
                                    .contextMenu {
                                        Button {
                                            coverPickerWork = cw
                                        } label: {
                                            Label("更换封面", systemImage: "photo.on.rectangle")
                                        }
                                        Button {
                                            openDetail(cw)
                                        } label: {
                                            Label("查看详情", systemImage: "info.circle")
                                        }
                                    }
                                }
                            }
                            .padding(.horizontal, 14)
                        }
                    }
                }

                if !selectedCategory.isEmpty || sortMode != nil || !selectedAuthor.isEmpty || !selectedParody.isEmpty || !selectedTag.isEmpty || !selectedYear.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            if !selectedCategory.isEmpty {
                                Button { selectedCategory = ""; Task { await loadShelf(reset: true) } } label: {
                                    Label("类型：\(selectedCategory)", systemImage: "xmark.circle.fill").font(.caption.bold())
                                }.buttonStyle(.borderedProminent).controlSize(.mini).tint(.purple)
                            }
                            if let sortMode, !sortMode.isEmpty {
                                Button { self.sortMode = nil; Task { await loadShelf(reset: true) } } label: {
                                    Label(sortLabel(sortMode), systemImage: "xmark.circle.fill").font(.caption.bold())
                                }.buttonStyle(.borderedProminent).controlSize(.mini).tint(.indigo)
                            }
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
                                PMLSkeletonBlock(cornerRadius: 8)
                                    .aspectRatio(2.0 / 3.0, contentMode: .fit)
                                PMLSkeletonBlock(cornerRadius: 4)
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
                                        Color.secondary.opacity(0.12)
                                            .aspectRatio(2.0 / 3.0, contentMode: .fit)
                                            .overlay {
                                                CachedImageView(
                                                    url: coverURL(assetId: work.coverAssetId, isArchive: work.coverIsArchive, pageIndex: work.coverPageIndex ?? 0),
                                                    contentMode: .fill,
                                                    minHeight: 0,
                                                    maxPixel: 420
                                                )
                                            }
                                            .clipped()
                                            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                                        if work.volumeCount > 1 {
                                            Text("\(work.volumeCount)卷")
                                                .font(.system(size: 9, weight: .bold))
                                                .padding(.horizontal, 5)
                                                .padding(.vertical, 2)
                                                .background(Color.black.opacity(0.76), in: RoundedRectangle(cornerRadius: 4))
                                                .foregroundStyle(.white)
                                                .padding(5)
                                        } else if let tp = work.totalPages, tp > 0 {
                                            Text("\(tp)P")
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
                            .contextMenu {
                                Button {
                                    coverPickerWork = work
                                } label: {
                                    Label("更换封面", systemImage: "photo.on.rectangle")
                                }
                                Button {
                                    openDetail(work)
                                } label: {
                                    Label("查看详情", systemImage: "info.circle")
                                }
                                Button {
                                    Task { await favs.toggle(.comic, id: work.id, client: model.apiClient) }
                                } label: {
                                    Label(
                                        favs.isComicFav(work.id) ? "取消收藏" : "加入收藏",
                                        systemImage: favs.isComicFav(work.id) ? "heart.slash" : "heart"
                                    )
                                }
                            }
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
                        }
                    }
                    .padding(.vertical, 4)

                    // 独立整行操作栏（彻底避免在封面右侧窄列里挤压换行）
                    HStack(spacing: 8) {
                        if let targetVol = resumeVol {
                            Button {
                                openVolume(targetVol, workId: work.id)
                            } label: {
                                Label(workDetail?.work.lastReadVolumeLabel != nil ? "继续阅读" : "开始阅读", systemImage: "book.fill")
                                    .font(.subheadline.weight(.semibold))
                                    .lineLimit(1)
                                    .minimumScaleFactor(0.85)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 4)
                            }
                            .buttonStyle(.borderedProminent)
                            .tint(.purple)
                        }
                        Button {
                            sourceQuery = work.title
                            sourceCandidates = []
                            showSourceSheet = true
                            Task { await runManualSourceSearch(workId: work.id, useCoverHash: true) }
                        } label: {
                            Label("搜源", systemImage: "globe")
                                .font(.subheadline.weight(.semibold))
                                .lineLimit(1)
                                .minimumScaleFactor(0.85)
                                .padding(.vertical, 4)
                                .padding(.horizontal, 6)
                        }
                        .buttonStyle(.bordered)
                        Button {
                            Task { await favs.toggle(.comic, id: work.id, client: model.apiClient) }
                        } label: {
                            Image(systemName: favs.isComicFav(work.id) ? "heart.fill" : "heart")
                                .foregroundStyle(.pink)
                                .padding(.vertical, 4)
                                .padding(.horizontal, 4)
                        }
                        .buttonStyle(.bordered)
                    }
                    .padding(.vertical, 2)
                    if let summary = workDetail?.work.summary ?? work.summary, !summary.isEmpty {
                        Text(summary)
                            .font(.caption)
                            .foregroundStyle(.secondary)
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

    private func sortLabel(_ raw: String) -> String {
        switch raw {
        case "recent_read": return "最近阅读"
        case "published_desc": return "最新出版"
        case "volumes_desc": return "卷数最多"
        case "title_asc": return "书名 A-Z"
        case "random": return "随机漫步"
        default: return "最新入库"
        }
    }

    private var filterSheetView: some View {
        NavigationStack {
            Form {
                Section("排序与随机") {
                    Picker("排序方式", selection: Binding(
                        get: { sortMode ?? "" },
                        set: { sortMode = $0.isEmpty ? nil : $0 }
                    )) {
                        Text("最新入库").tag("")
                        Text("最近阅读").tag("recent_read")
                        Text("最新出版").tag("published_desc")
                        Text("卷数最多").tag("volumes_desc")
                        Text("书名 A-Z").tag("title_asc")
                        Text("随机漫步").tag("random")
                    }
                    Button {
                        sortMode = "random"
                        showFilterSheet = false
                        Task { await loadShelf(reset: true) }
                    } label: {
                        Label("随机漫步（换一批随机顺序）", systemImage: "shuffle")
                    }
                }
                if let cats = filterOptions?.categories, !cats.isEmpty {
                    Section("漫画类型") {
                        Picker("类型", selection: $selectedCategory) {
                            Text("全部类型").tag("")
                            ForEach(cats) { c in
                                Text("\(c.name) (\(c.count))").tag(c.name)
                            }
                        }
                    }
                }
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
        let prevWorks = stats?.totalWorks ?? 0
        let prevVolumes = stats?.totalVolumes ?? 0
        busySync = true
        dataFeedback = "正在增量同步并自动聚类建库…"
        defer { busySync = false }
        do {
            let scanRes = try await client.scanLibraryModule(moduleId: "comics", full: false)
            let st = try await client.rebuildComicLibrary()
            stats = st
            let dWorks = st.totalWorks - prevWorks
            let dVols = st.totalVolumes - prevVolumes
            let deltaText = dWorks > 0 || dVols > 0
                ? "本次新增 +\(max(0, dWorks)) 部作品 / +\(max(0, dVols)) 卷"
                : "暂无新增作品"
            dataFeedback = "增量同步完成：新增文件 +\(scanRes.scan.added) · 更新 \(scanRes.scan.updated) · 删除 -\(scanRes.scan.deleted)；\(deltaText)（现有 \(st.totalWorks) 部 / \(st.totalSeries) 系列 / \(st.totalVolumes) 卷）"
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

// MARK: - 在线漫画源（禁漫天堂 JM + 哔咔漫画 PicACG）

private struct OnlineComicsPaneView: View {
    @EnvironmentObject private var model: AppModel
    @Binding var showFilterSheet: Bool

    @State private var source = "jm"
    @State private var mode = "browse"
    @State private var query = ""
    @State private var category = "全部"
    @State private var categories: [String] = ["全部", "同人", "单本", "短篇", "韩漫", "美漫", "其他"]
    @State private var sort = "mr"
    @State private var page = 1
    @State private var hasMore = false
    @State private var loading = false
    @State private var errorMsg = ""
    @State private var items: [OnlineComicItem] = []

    @State private var config: OnlineComicConfigView?
    @State private var showConfigSheet = false
    @State private var cfgJmDomains = ""
    @State private var cfgJmImgDomains = ""
    @State private var cfgPicaEmail = ""
    @State private var cfgPicaPassword = ""
    @State private var cfgPicaChannel = "1"
    @State private var cfgFeedback = ""

    @State private var selectedItem: OnlineComicItem?
    @State private var detail: OnlineComicDetail?
    @State private var detailLoading = false
    @State private var chapterDesc = false
    @State private var activeChapter: OnlineComicChapter?
    @State private var readerImages: [String] = []
    @State private var readerLoading = false

    // Magnet search & CBZ packaging
    @State private var showMagnetSheet = false
    @State private var magnetQuery = ""
    @State private var magnetLoading = false
    @State private var magnetItems: [UniversalMagnetItem] = []
    @State private var copiedMagnetHash = ""
    @State private var magnetFeedback = ""
    @ObservedObject private var offlineStore = OfflineHistoryStore.shared
    @State private var filesTarget: MagnetFilesSheetTarget?
    @State private var cbzTask: CbzPackageTask?
    @State private var cbzPollTask: Task<Void, Never>?

    private let columns = [
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
        GridItem(.flexible(), spacing: 10),
    ]

    private var commonCategories: [String] {
        let preferred = ["全部", "同人", "单本", "短篇", "韩漫", "美漫", "全彩", "CG"]
        var picked = preferred.filter { categories.contains($0) }
        for c in categories where picked.count < 7 && !picked.contains(c) {
            picked.append(c)
        }
        if !category.isEmpty && !picked.contains(category) {
            picked.insert(category, at: min(1, picked.count))
        }
        return picked.isEmpty ? Array(categories.prefix(7)) : picked
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 8) {
                    TextField(source == "jm" ? "搜索书名、作者或 JM 号…" : "搜索哔咔书名/作者…", text: $query)
                        .textFieldStyle(.roundedBorder)
                        .submitLabel(.search)
                        .onSubmit {
                            mode = "browse"
                            Task { await loadList(reset: true) }
                        }
                    Button("搜索") {
                        mode = "browse"
                        Task { await loadList(reset: true) }
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(.pink)
                }
                .padding(.horizontal, 14)

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        Text(source == "jm" ? "禁漫" : "哔咔")
                            .font(.caption2.weight(.bold))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .background(Color.purple.opacity(0.18), in: Capsule())
                            .foregroundStyle(.purple)

                        if mode != "browse" {
                            Button {
                                mode = "browse"
                                Task { await loadList(reset: true) }
                            } label: {
                                Label(modeDisplayLabel(mode), systemImage: "xmark.circle.fill")
                                    .font(.caption.weight(.semibold))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 5)
                                    .background(Color.purple, in: Capsule())
                                    .foregroundStyle(.white)
                            }
                        }

                        ForEach(commonCategories, id: \.self) { c in
                            Button {
                                mode = "browse"
                                category = c
                                Task { await loadList(reset: true) }
                            } label: {
                                Text(c)
                                    .font(.caption.weight(.semibold))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background((mode == "browse" && category == c) ? Color.pink : Color.secondary.opacity(0.14), in: Capsule())
                                    .foregroundStyle((mode == "browse" && category == c) ? .white : .primary)
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                }

                if !errorMsg.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        Text(errorMsg).font(.caption).foregroundStyle(.red)
                        if source == "pica" && config?.picaLoggedIn != true {
                            Button("配置哔咔账号") { showConfigSheet = true }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                        }
                    }
                    .padding(.horizontal, 14)
                }

                if loading && items.isEmpty {
                    ProgressView("正在拉取在线漫画…")
                        .frame(maxWidth: .infinity)
                        .padding(.top, 40)
                } else {
                    LazyVGrid(columns: columns, spacing: 12) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { idx, item in
                            Button {
                                openDetail(item)
                            } label: {
                                VStack(alignment: .leading, spacing: 5) {
                                    ZStack(alignment: .topLeading) {
                                        Color.secondary.opacity(0.12)
                                            .aspectRatio(2.0 / 3.0, contentMode: .fit)
                                            .overlay {
                                                CachedImageView(
                                                    url: model.apiClient?.onlineComicImageURL(relativePath: item.coverUrl),
                                                    contentMode: .fill,
                                                    minHeight: 0,
                                                    maxPixel: 420
                                                )
                                            }
                                            .clipped()
                                            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                                        if mode.hasPrefix("rank_") {
                                            Text("#\(idx + 1)")
                                                .font(.system(size: 10, weight: .heavy))
                                                .padding(.horizontal, 6)
                                                .padding(.vertical, 2)
                                                .background(idx < 3 ? Color.orange : Color.black.opacity(0.75), in: RoundedRectangle(cornerRadius: 5))
                                                .foregroundStyle(.white)
                                                .padding(5)
                                        }
                                    }
                                    Text(item.title)
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(.primary)
                                        .lineLimit(2)
                                        .multilineTextAlignment(.leading)
                                    Text(item.author.isEmpty ? (item.source == "jm" ? "JM\(item.comicId)" : "哔咔") : item.author)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }
                            }
                            .buttonStyle(.plain)
                            .onAppear {
                                if item.id == items.last?.id && hasMore && !loading {
                                    Task { await loadList(reset: false) }
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 14)

                    if hasMore {
                        Button("加载下一页") { Task { await loadList(reset: false) } }
                            .buttonStyle(.bordered)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 8)
                    }
                }
            }
            .padding(.vertical, 8)
        }
        .refreshable {
            await loadList(reset: true)
        }
        .onDisappear { cbzPollTask?.cancel() }
        .task {
            await loadConfig()
            await loadCategories()
            await loadList(reset: true)
        }
        .sheet(isPresented: $showFilterSheet) {
            NavigationStack {
                Form {
                    Section("漫画源") {
                        Picker("漫画源", selection: $source) {
                            Text("禁漫 (JM)").tag("jm")
                            Text("哔咔 (PicACG)").tag("pica")
                        }
                        .pickerStyle(.segmented)
                        .onChange(of: source) { _, newVal in
                            mode = "browse"
                            category = "全部"
                            sort = (newVal == "jm" ? "mr" : "dd")
                            Task {
                                await loadCategories()
                                await loadList(reset: true)
                            }
                        }
                    }

                    Section("频道模式") {
                        Picker("模式", selection: $mode) {
                            Text("分类发现").tag("browse")
                            Text("日排行").tag("rank_day")
                            Text("周排行").tag("rank_week")
                            Text("月排行").tag("rank_month")
                            if source == "jm" { Text("总排行").tag("rank_all") }
                            Text("随机推荐").tag("random")
                            if source == "pica" { Text("我的哔咔收藏").tag("favorites") }
                        }
                        .onChange(of: mode) { _, _ in
                            Task { await loadList(reset: true) }
                        }
                    }

                    if mode == "browse" || mode == "favorites" {
                        Section("排序") {
                            Picker("排序", selection: $sort) {
                                if source == "jm" {
                                    Text("最新更新").tag("mr")
                                    Text("最多点击").tag("mv")
                                    Text("最多爱心").tag("tf")
                                    Text("最多图片").tag("mp")
                                } else {
                                    Text("最新上架").tag("dd")
                                    if mode == "browse" {
                                        Text("最多爱心").tag("ld")
                                        Text("最多指名").tag("vd")
                                    }
                                    Text("最早发布").tag("da")
                                }
                            }
                            .onChange(of: sort) { _, _ in
                                Task { await loadList(reset: true) }
                            }
                        }
                    }

                    if mode == "browse" {
                        Section("全部分类") {
                            Picker("分类", selection: $category) {
                                ForEach(categories, id: \.self) { c in
                                    Text(c).tag(c)
                                }
                            }
                            .onChange(of: category) { _, _ in
                                Task { await loadList(reset: true) }
                            }
                        }
                    }

                    Section {
                        Button {
                            showFilterSheet = false
                            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                                showConfigSheet = true
                            }
                        } label: {
                            Label("源账号与域名配置", systemImage: "gearshape")
                        }
                    }
                }
                .navigationTitle("在线漫画 · 源与筛选")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("完成") { showFilterSheet = false }
                    }
                }
            }
            .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $showConfigSheet) {
            NavigationStack {
                Form {
                    Section("哔咔漫画 (PicACG) 账号") {
                        TextField("邮箱 / 账号", text: $cfgPicaEmail)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        SecureField(config?.picaHasPassword == true ? "密码（已保存，留空不变）" : "密码", text: $cfgPicaPassword)
                        Picker("分流线路", selection: $cfgPicaChannel) {
                            Text("分流 1").tag("1")
                            Text("分流 2").tag("2")
                            Text("分流 3").tag("3")
                        }
                    }
                    Section("禁漫天堂 (JM) 域名池（逗号分隔）") {
                        TextField("API 域名", text: $cfgJmDomains)
                            .textInputAutocapitalization(.never)
                        TextField("图片 CDN 域名", text: $cfgJmImgDomains)
                            .textInputAutocapitalization(.never)
                    }
                    if !cfgFeedback.isEmpty {
                        Section { Text(cfgFeedback).font(.caption).foregroundStyle(.purple) }
                    }
                    Section {
                        Button("保存并测试登录") { Task { await saveConfig() } }
                    }
                }
                .navigationTitle("在线漫画源设置")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("完成") { showConfigSheet = false }
                    }
                }
            }
        }
        .sheet(item: $selectedItem) { item in
            NavigationStack {
                let chs = detail?.chapters ?? []
                let orderedChs = chapterDesc ? Array(chs.reversed()) : chs
                List {
                    Section {
                        HStack(alignment: .top, spacing: 12) {
                            CachedImageView(
                                url: model.apiClient?.onlineComicImageURL(relativePath: item.coverUrl),
                                contentMode: .fill,
                                minHeight: 148,
                                maxPixel: 420
                            )
                            .frame(width: 100, height: 150)
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            VStack(alignment: .leading, spacing: 6) {
                                Text(detail?.title ?? item.title).font(.headline)
                                if let auths = detail?.authors, !auths.isEmpty {
                                    Text("作者：\(auths.joined(separator: " / "))").font(.caption).foregroundStyle(.purple)
                                }
                                if let desc = detail?.description, !desc.isEmpty {
                                    Text(desc).font(.caption2).foregroundStyle(.secondary).lineLimit(4)
                                }
                            }
                        }

                        if let tags = detail?.tags, !tags.isEmpty {
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 6) {
                                    ForEach(tags.prefix(16), id: \.self) { t in
                                        Text("#\(t)")
                                            .font(.caption2)
                                            .padding(.horizontal, 8)
                                            .padding(.vertical, 4)
                                            .background(Color.purple.opacity(0.14), in: Capsule())
                                            .foregroundStyle(.purple)
                                    }
                                }
                            }
                        }

                        // 核心操作区：开始阅读 / 搜磁力 / 打包 CBZ / 哔咔收藏
                        HStack(spacing: 8) {
                            if let firstCh = chs.first {
                                Button {
                                    openChapter(comicId: detail?.id ?? item.comicId, chapter: firstCh)
                                } label: {
                                    Label("开始阅读", systemImage: "book.fill")
                                }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                                .tint(.purple)
                            }
                            Button {
                                magnetQuery = detail?.title ?? item.title
                                magnetItems = []
                                magnetFeedback = ""
                                showMagnetSheet = true
                                Task {
                                    await offlineStore.ensureLoaded(client: model.apiClient)
                                    await runMagnetSearch()
                                }
                            } label: {
                                Label("搜磁力", systemImage: "link")
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)

                            if let firstCh = chs.first {
                                Button {
                                    Task {
                                        await startCbzPackage(
                                            comicId: detail?.id ?? item.comicId,
                                            chapter: firstCh,
                                            title: detail?.title ?? item.title,
                                            author: detail?.authors.first
                                        )
                                    }
                                } label: {
                                    Label("CBZ", systemImage: "arrow.down.doc")
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                                .disabled(cbzTask?.status == "running")
                            }

                            if item.source == "pica" {
                                Button {
                                    Task { await togglePicaFav(comicId: detail?.id ?? item.comicId) }
                                } label: {
                                    Image(systemName: detail?.isFavourite == true ? "heart.fill" : "heart")
                                        .foregroundStyle(.pink)
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                            }
                        }

                        if let task = cbzTask {
                            VStack(alignment: .leading, spacing: 6) {
                                if task.status == "running" {
                                    ProgressView(value: Double(task.completed), total: Double(max(task.total, 1)))
                                        .tint(.purple)
                                    Text("正在打包 CBZ：\(task.completed) / \(task.total) 页…")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                } else if task.status == "done", let dl = task.downloadUrl, let url = model.apiClient?.onlineComicImageURL(relativePath: dl) {
                                    HStack {
                                        Label("CBZ 已生成", systemImage: "checkmark.circle.fill")
                                            .font(.caption.bold())
                                            .foregroundStyle(.green)
                                        Spacer()
                                        ShareLink(item: url) {
                                            Label("保存到文件", systemImage: "square.and.arrow.up")
                                                .font(.caption.bold())
                                        }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.mini)
                                        .tint(.purple)
                                    }
                                } else if let err = task.error {
                                    Text("打包失败：\(err)").font(.caption2).foregroundStyle(.red)
                                }
                            }
                            .padding(.vertical, 4)
                        }
                    }

                    if detailLoading {
                        Section { ProgressView("加载章节中…") }
                    } else if !orderedChs.isEmpty {
                        Section {
                            ForEach(orderedChs) { ch in
                                Button {
                                    openChapter(comicId: detail?.id ?? item.comicId, chapter: ch)
                                } label: {
                                    HStack {
                                        Text(ch.title).foregroundStyle(.primary)
                                        Spacer()
                                        Image(systemName: "chevron.right").font(.caption2).foregroundStyle(.tertiary)
                                    }
                                }
                                .contextMenu {
                                    Button {
                                        Task {
                                            await startCbzPackage(
                                                comicId: detail?.id ?? item.comicId,
                                                chapter: ch,
                                                title: "\(detail?.title ?? item.title) - \(ch.title)",
                                                author: detail?.authors.first
                                            )
                                        }
                                    } label: {
                                        Label("打包本话为 CBZ", systemImage: "arrow.down.doc")
                                    }
                                }
                            }
                        } header: {
                            HStack {
                                Text("章节目录（\(chs.count) 话）")
                                Spacer()
                                if chs.count > 1 {
                                    Button(chapterDesc ? "倒序" : "正序") {
                                        chapterDesc.toggle()
                                    }
                                    .font(.caption)
                                }
                            }
                        }
                    }
                }
                .navigationTitle("在线漫画详情")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("关闭") { selectedItem = nil }
                    }
                }
                .sheet(isPresented: $showMagnetSheet) {
                    NavigationStack {
                        List {
                            Section {
                                HStack(spacing: 8) {
                                    TextField("输入日文/中文书名或作者…", text: $magnetQuery)
                                        .textFieldStyle(.roundedBorder)
                                        .submitLabel(.search)
                                        .onSubmit { Task { await runMagnetSearch() } }
                                    Button("搜索") { Task { await runMagnetSearch() } }
                                        .buttonStyle(.borderedProminent)
                                        .tint(.purple)
                                }
                                if !magnetFeedback.isEmpty {
                                    Text(magnetFeedback)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            if magnetLoading {
                                Section { ProgressView("正在搜索 Sukebei / Nyaa 磁力…") }
                            } else if magnetItems.isEmpty {
                                Section {
                                    Text("未搜到磁力链接，可缩短书名关键词重试，或直接点「CBZ」打包保存。")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            } else {
                                Section("磁力结果（\(magnetItems.count)）") {
                                    ForEach(magnetItems) { m in
                                        let isOff = offlineStore.isOfflined(m.infoHash) || offlineStore.isOfflined(m.magnet)
                                        VStack(alignment: .leading, spacing: 6) {
                                            Text(m.title).font(.caption.weight(.semibold))
                                            HStack(spacing: 8) {
                                                Text(m.tracker)
                                                    .font(.caption2.bold())
                                                    .padding(.horizontal, 6)
                                                    .padding(.vertical, 2)
                                                    .background(Color.blue.opacity(0.16), in: Capsule())
                                                    .foregroundStyle(.blue)
                                                Text(m.sizeText).font(.caption2).foregroundStyle(.secondary)
                                                Text("做种 \(m.seeders)").font(.caption2).foregroundStyle(.green)
                                                Spacer()
                                                Button {
                                                    Task {
                                                        guard let client = model.apiClient else { return }
                                                        do {
                                                            let task = try await offlineStore.submit(client: client, magnet: m.magnet, title: m.title)
                                                            magnetFeedback = "已加入 115 离线下载：\(task.targetDir)"
                                                        } catch {
                                                            magnetFeedback = "离线失败：\(error.localizedDescription)"
                                                        }
                                                    }
                                                } label: {
                                                    if offlineStore.busyKey == OfflineHistoryStore.cleanURL(m.magnet).lowercased() {
                                                        ProgressView().scaleEffect(0.65)
                                                    } else {
                                                        Label(isOff ? "已离线" : "离线", systemImage: isOff ? "checkmark.icloud.fill" : "icloud.and.arrow.down")
                                                            .font(.caption2.bold())
                                                    }
                                                }
                                                .buttonStyle(.borderedProminent)
                                                .controlSize(.mini)
                                                .tint(isOff ? .green : .blue)
                                                .disabled(!offlineStore.busyKey.isEmpty)

                                                Button {
                                                    filesTarget = MagnetFilesSheetTarget(infoHash: m.infoHash, title: m.title)
                                                } label: {
                                                    Image(systemName: "folder").font(.caption2)
                                                }
                                                .buttonStyle(.bordered)
                                                .controlSize(.mini)

                                                Button {
                                                    UIPasteboard.general.string = m.magnet
                                                    copiedMagnetHash = m.infoHash
                                                } label: {
                                                    Label(
                                                        copiedMagnetHash == m.infoHash ? "已复制" : "复制",
                                                        systemImage: copiedMagnetHash == m.infoHash ? "checkmark" : "doc.on.doc"
                                                    )
                                                    .font(.caption2.bold())
                                                }
                                                .buttonStyle(.borderedProminent)
                                                .controlSize(.mini)
                                                .tint(copiedMagnetHash == m.infoHash ? .green : .purple)
                                            }
                                        }
                                        .padding(.vertical, 2)
                                    }
                                }
                            }
                        }
                        .navigationTitle("搜索漫画磁力")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("完成") { showMagnetSheet = false }
                            }
                        }
                        .sheet(item: $filesTarget) { t in
                            MagnetFilesSheetView(target: t).environmentObject(model)
                        }
                    }
                }
            }
        }
        .fullScreenCover(item: $activeChapter) { ch in
            let chs = detail?.chapters ?? []
            let readerChapters = chs.map { c in
                ComicReaderChapterItem(id: c.id, title: c.title, active: c.id == ch.id)
            }
            let pageURLs = readerImages.compactMap { model.apiClient?.onlineComicImageURL(relativePath: $0) }
            let readerTitle = chs.count > 1 ? "\(detail?.title ?? "") · \(ch.title)" : (detail?.title ?? ch.title)
            if readerLoading {
                ZStack {
                    Color.black.ignoresSafeArea()
                    ProgressView("加载图片并自动解混淆…")
                        .tint(.white)
                        .foregroundStyle(.white)
                }
            } else {
                ComicGalleryReaderView(
                    kind: .comic,
                    title: readerTitle,
                    pageURLs: pageURLs,
                    chapters: readerChapters,
                    onSelectChapter: { nextId in
                        if let nextCh = chs.first(where: { $0.id == nextId }) {
                            openChapter(comicId: detail?.id ?? "", chapter: nextCh)
                        }
                    },
                    onDismiss: { activeChapter = nil }
                )
                .environmentObject(model)
            }
        }
    }

    private func runMagnetSearch() async {
        guard let client = model.apiClient else { return }
        let q = magnetQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return }
        magnetLoading = true
        defer { magnetLoading = false }
        magnetItems = (try? await client.searchAllMagnets(query: q, category: "all", sort: "seeders", limit: 60))?.items ?? []
    }

    private func startCbzPackage(comicId: String, chapter: OnlineComicChapter, title: String, author: String?) async {
        guard let client = model.apiClient else { return }
        cbzPollTask?.cancel()
        if let started = try? await client.startOnlineComicCbz(
            source: source,
            comicId: comicId,
            chapterId: chapter.id,
            title: title,
            author: author
        ) {
            cbzTask = started
            let tid = started.taskId
            cbzPollTask = Task {
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(1.2))
                    if let st = try? await client.getOnlineComicCbzStatus(taskId: tid) {
                        cbzTask = st
                        if st.status != "running" { break }
                    } else {
                        break
                    }
                }
            }
        }
    }

    private func modeDisplayLabel(_ m: String) -> String {
        switch m {
        case "rank_day": return "日排行"
        case "rank_week": return "周排行"
        case "rank_month": return "月排行"
        case "rank_all": return "总排行"
        case "random": return "随机"
        case "favorites": return "我的收藏"
        default: return "发现"
        }
    }

    private func loadCategories() async {
        guard let client = model.apiClient else { return }
        if let list = try? await client.listOnlineComicCategories(source: source), !list.isEmpty {
            categories = list
        }
    }

    private func togglePicaFav(comicId: String) async {
        guard let client = model.apiClient else { return }
        if let isFav = try? await client.toggleOnlineComicFavorite(source: "pica", comicId: comicId), let d = detail {
            detail = OnlineComicDetail(
                source: d.source,
                id: d.id,
                title: d.title,
                authors: d.authors,
                coverUrl: d.coverUrl,
                description: d.description,
                tags: d.tags,
                isFavourite: isFav,
                chapters: d.chapters
            )
            if mode == "favorites" {
                await loadList(reset: true)
            }
        }
    }

    private func loadConfig() async {
        guard let client = model.apiClient else { return }
        if let c = try? await client.getOnlineComicConfig() {
            config = c
            cfgJmDomains = c.jmDomains.joined(separator: ",")
            cfgJmImgDomains = c.jmImageDomains.joined(separator: ",")
            cfgPicaEmail = c.picaEmail
            cfgPicaChannel = c.picaChannel
        }
    }

    private func saveConfig() async {
        guard let client = model.apiClient else { return }
        do {
            let jm = cfgJmDomains.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            let jmImg = cfgJmImgDomains.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
            let updated = try await client.saveOnlineComicConfig(
                jmDomains: jm,
                jmImageDomains: jmImg,
                picaEmail: cfgPicaEmail.trimmingCharacters(in: .whitespaces),
                picaPassword: cfgPicaPassword.isEmpty ? nil : cfgPicaPassword,
                picaChannel: cfgPicaChannel
            )
            config = updated
            cfgPicaPassword = ""
            cfgFeedback = updated.picaLoggedIn ? "已保存，哔咔登录成功！" : "配置已保存！"
            await loadList(reset: true)
        } catch {
            cfgFeedback = "保存失败：\(error.localizedDescription)"
        }
    }

    private func loadList(reset: Bool) async {
        guard let client = model.apiClient else { return }
        if reset {
            loading = true
            page = 1
            errorMsg = ""
        } else {
            page += 1
        }
        defer { loading = false }
        do {
            let res = try await client.browseOnlineComics(
                source: source,
                mode: mode,
                query: mode == "browse" ? query.trimmingCharacters(in: .whitespacesAndNewlines) : nil,
                category: mode == "browse" ? category : nil,
                sort: sort,
                page: page
            )
            items = reset ? res.items : (items + res.items)
            hasMore = res.hasMore
        } catch {
            errorMsg = error.localizedDescription
            if reset { items = [] }
        }
    }

    private func openDetail(_ item: OnlineComicItem) {
        selectedItem = item
        detail = nil
        detailLoading = true
        chapterDesc = false
        cbzTask = nil
        cbzPollTask?.cancel()
        Task {
            guard let client = model.apiClient else { return }
            defer { detailLoading = false }
            detail = try? await client.getOnlineComicDetail(source: item.source, comicId: item.comicId)
        }
    }

    private func openChapter(comicId: String, chapter: OnlineComicChapter) {
        selectedItem = nil
        activeChapter = chapter
        readerImages = []
        readerLoading = true
        Task {
            guard let client = model.apiClient else { return }
            defer { readerLoading = false }
            if let res = try? await client.getOnlineComicChapterPages(source: source, comicId: comicId, chapterId: chapter.id) {
                readerImages = res.images
            }
        }
    }
}

private struct ComicCoverPickerSheet: View {
    let work: ComicWorkItem
    let onSaved: (ComicWorkItem) -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var loading = true
    @State private var saving = false
    @State private var errorText = ""
    @State private var firstVolume: ComicVolumeItem?
    @State private var archivePageCount = 0
    @State private var folderPages: [RemoteBrowserEntry] = []

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
                    } else if let vol = firstVolume, vol.isArchive {
                        let count = max(1, min(archivePageCount, 18))
                        LazyVGrid(columns: columns, spacing: 10) {
                            ForEach(0..<count, id: \.self) { pIdx in
                                let isCurrent = (work.coverPageIndex ?? 0) == pIdx && work.coverAssetId == vol.assetId
                                Button {
                                    Task { await saveCover(assetId: vol.assetId, pageIndex: pIdx) }
                                } label: {
                                    ZStack(alignment: .bottomTrailing) {
                                        CachedImageView(
                                            url: model.apiClient?.comicPageURL(assetId: vol.assetId, pageIndex: pIdx, thumb: true),
                                            contentMode: .fill,
                                            minHeight: 130,
                                            maxPixel: 360
                                        )
                                        .aspectRatio(2.0 / 3.0, contentMode: .fit)
                                        .clipped()
                                        .clipShape(RoundedRectangle(cornerRadius: 8))
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 8)
                                                .stroke(isCurrent ? Color.purple : Color.clear, lineWidth: 2.5)
                                        )
                                        Text(isCurrent ? "当前" : "P\(pIdx + 1)")
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
                    } else if !folderPages.isEmpty {
                        LazyVGrid(columns: columns, spacing: 10) {
                            ForEach(Array(folderPages.prefix(18).enumerated()), id: \.element.id) { idx, entry in
                                let aid = entry.assetId ?? ""
                                let isCurrent = !aid.isEmpty && work.coverAssetId == aid
                                Button {
                                    if !aid.isEmpty {
                                        Task { await saveCover(assetId: aid, pageIndex: 0) }
                                    }
                                } label: {
                                    ZStack(alignment: .bottomTrailing) {
                                        CachedImageView(
                                            url: model.apiClient?.v2CoverURL(assetId: entry.assetId, isArchive: false),
                                            contentMode: .fill,
                                            minHeight: 130,
                                            maxPixel: 360
                                        )
                                        .aspectRatio(2.0 / 3.0, contentMode: .fit)
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
                                .disabled(saving || aid.isEmpty)
                            }
                        }
                    } else {
                        ContentUnavailableView("暂无可选封面页", systemImage: "photo")
                    }
                }
                .padding(14)
            }
            .navigationTitle("选择漫画封面")
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
        guard let detail = try? await client.getComicWorkDetail(workId: work.id),
              let vol = detail.volumes.first else {
            return
        }
        firstVolume = vol
        if vol.isArchive {
            if let info = try? await client.inspectComicArchive(assetId: vol.assetId) {
                archivePageCount = info.pages.count
            } else {
                archivePageCount = Int(max(6, min(vol.pageCount, 18)))
            }
        } else if let folder = vol.folderPath {
            let roots = (try? await client.libraryRoots()) ?? []
            let root = roots.first(where: { $0.kind == "comics" })
                ?? LibraryRoot(id: "comics", kind: "comics", remotePath: folder, createdAt: 0)
            if let page = try? await client.remoteChildren(rootId: root.id, path: folder) {
                folderPages = page.items.filter(\.isImage)
            }
        }
    }

    private func saveCover(assetId: String, pageIndex: Int) async {
        guard let client = model.apiClient, !saving else { return }
        saving = true
        defer { saving = false }
        do {
            let updated = try await client.setComicWorkCover(workId: work.id, pageIndex: pageIndex, assetId: assetId)
            onSaved(updated)
            dismiss()
        } catch {
            errorText = "设置封面失败：\(error.localizedDescription)"
        }
    }
}
