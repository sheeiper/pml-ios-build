import SwiftUI

/// Server-backed movie library. Search, facets, sorting and pagination are never performed locally.
public struct MovieLibraryView: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var favs = EntityFavoritesStore.shared
    @State private var pageItems: [MovieMetadata] = []
    @State private var total = 0
    @State private var makers: [String] = []
    @State private var actors: [String] = []
    @State private var randomHeroPool: [MovieMetadata] = []
    @State private var heroIndex = 0
    @State private var topPerformers: [MoviePerformer] = []
    @State private var issues: [ScrapeIssue] = []
    @State private var issueCounts: [ScrapeIssueCount] = []
    @State private var issueTotal = 0
    @State private var unrecognized: [UnrecognizedVideo] = []
    @State private var unrecognizedTotal = 0
    @State private var performers: [MoviePerformer] = []
    @State private var performerTotal = 0
    @State private var searchQuery = ""
    @State private var issueCategory = ""
    @State private var selectedMaker = ""
    @State private var selectedActor = ""
    @State private var onlyFavMovies = false
    @State private var onlyFavPerformers = false
    @State private var sort = "release_date_desc"
    @State private var performerSort = "latest_release"
    @State private var performerAgeRange = ""
    @State private var performerHeightRange = ""
    @State private var performerCup = ""
    @State private var performerMinMovies = 0
    @State private var performerProfileFilter = ""
    @State private var mode: LibraryMode = .movies
    @State private var offset = 0
    @State private var loading = false
    @State private var loadingMore = false
    @State private var errorMessage: String?
    @State private var selectedMovie: MovieMetadata?
    @State private var selectedPerformer: MoviePerformer?
    @State private var preview: PreviewItem?
    @State private var showFilterDrawer = false
    @State private var showIssuesSection = false
    @State private var autoFlowRunning = false
    @State private var autoFlowNote: String? = nil
    @State private var onlineSearchPreset = ""
    @State private var pendingFilter: PendingSubFilter = .all
    @State private var issueOffset = 0
    @State private var unrecognizedOffset = 0
    @State private var batchStats: BatchScrapeStats?
    @State private var batchProgress: BatchScrapeProgress?
    @State private var enrichStats: PerformerEnrichStats?
    @State private var enrichProgress: PerformerEnrichProgress?
    @State private var pipelineDiff: MovieSyncDiffSummary?
    @State private var pipelineSyncProgress: MovieDbSyncProgress?
    @State private var pipelineExtract: MovieExtractSummary?
    @State private var pipelineSyncMode: String?
    @State private var syncPollTask: Task<Void, Never>?
    @State private var pipelineExtracting = false
    @State private var showDiffAdded = false
    @State private var showDiffRemoved = false
    @State private var manualTargetVideo: UnrecognizedVideo?
    @State private var manualNumberInput = ""
    @State private var scrapePollTask: Task<Void, Never>?
    @State private var requestVersion = 0

    private let pageSize = 40
    private enum LibraryMode: String, CaseIterable, Identifiable {
        case movies = "影视库", performers = "演员", pending = "数据"
        var id: String { rawValue }
    }
    private enum PendingSubFilter: String, CaseIterable, Identifiable {
        case all = "全部", failed = "刮削失败", unrecognized = "未识别"
        var id: String { rawValue }
    }

    public init() {}

    private var hasActiveFilters: Bool {
        !searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !selectedMaker.isEmpty || !selectedActor.isEmpty || onlyFavMovies
    }

    private var heroMovie: MovieMetadata? {
        guard !randomHeroPool.isEmpty else { return nil }
        return randomHeroPool[heroIndex % randomHeroPool.count]
    }

    public var body: some View {
        ZStack {
            if let performer = selectedPerformer {
                MoviePerformerDetailSheet(
                    performer: performer,
                    onBack: { selectedPerformer = nil },
                    onFilterMovies: { actorName in
                        selectedPerformer = nil
                        mode = .movies
                        selectedActor = actorName
                        resetAndLoad()
                    },
                    onPlay: { assetId, title in
                        Task {
                            try? await Task.sleep(nanoseconds: 420_000_000)
                            await playAsset(assetId, title: title)
                        }
                    }
                )
                .environmentObject(model)
            } else {
                VStack(spacing: 0) {
                    Picker("内容", selection: $mode) {
                        ForEach(LibraryMode.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .padding(.horizontal)
                    .padding(.top, 8)
                    .onChange(of: mode) { _, _ in resetAndLoad() }

                    if mode == .pending {
                        dataConsoleView
                    } else {
                        if mode == .movies, activeMovieFilterCount > 0 { activeMovieFilterStrip }
                        else if mode == .performers, activePerformerFilterCount > 0 { activePerformerFilterStrip }

                        if let errorMessage {
                            Label(errorMessage, systemImage: "exclamationmark.triangle")
                                .font(.footnote).foregroundStyle(.red)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.horizontal).padding(.vertical, 8)
                        }

                        content
                    }
                }
                .navigationTitle(
                    mode == .movies ? "影视库 (\(total))"
                    : mode == .pending ? "数据管理"
                    : mode.rawValue
                )
                .navigationBarTitleDisplayMode(.inline)
                .searchable(text: $searchQuery, prompt: mode == .performers ? "搜索演员姓名或别名" : "搜索番号、片名、演员或片厂")
                .toolbar {
                    ToolbarItemGroup(placement: .topBarTrailing) {
                        if mode == .movies || mode == .performers {
                            let count = mode == .movies ? activeMovieFilterCount : activePerformerFilterCount
                            Button { showFilterDrawer = true } label: {
                                HStack(spacing: 4) {
                                    Image(systemName: count > 0 ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
                                    if count > 0 {
                                        Text("\(count)")
                                            .font(.caption2.bold())
                                    }
                                }
                                .foregroundStyle(count > 0 ? .purple : .primary)
                            }
                        }
                    }
                }
                .task {
                    await favs.ensureLoaded(client: model.apiClient)
                    await reload()
                    await loadDiscoveryData()
                }
                .task(id: searchQuery) {
                    // Debounce both typing and clearing the search field so an empty query reloads the full server page.
                    try? await Task.sleep(for: .milliseconds(350))
                    guard !Task.isCancelled else { return }
                    await reload()
                }
                .refreshable {
                    await reload()
                    if mode == .movies { await loadDiscoveryData() }
                    else if mode == .pending { await refreshConsoleState() }
                }
                .onDisappear { scrapePollTask?.cancel(); syncPollTask?.cancel() }
                .sheet(item: $selectedMovie) { movie in
                    MovieDetailSheet(
                        number: movie.number,
                        assetId: movie.assetId,
                        fileName: movie.title,
                        remotePath: nil,
                        onPlay: { target in
                            selectedMovie = nil
                            selectedPerformer = nil
                            Task {
                                try? await Task.sleep(nanoseconds: 380_000_000)
                                await playAsset(target, title: movie.title)
                            }
                        },
                        onFilterActor: { actorName in
                            mode = .movies
                            selectedActor = actorName
                            resetAndLoad()
                        },
                        onFilterMaker: { makerName in
                            mode = .movies
                            selectedMaker = makerName
                            resetAndLoad()
                        }
                    )
                    .environmentObject(model)
                }
            }
        }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(url: item.url, title: item.title, assetId: item.assetId, startAt: item.startAt, fallbackURL: item.fallbackURL, sourceLabel: item.sourceLabel, remotePath: item.remotePath)
                .id(item.id).environmentObject(model)
        }
        .sheet(isPresented: $showFilterDrawer) {
            filterDrawerSheet
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
        .alert("指定番号并刮削", isPresented: Binding(
            get: { manualTargetVideo != nil },
            set: { if !$0 { manualTargetVideo = nil } }
        )) {
            TextField("例如 SSIS-123 或 FC2-4833053", text: $manualNumberInput)
                .textInputAutocapitalization(.characters)
            Button("开始刮削") {
                if let target = manualTargetVideo {
                    let num = manualNumberInput.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
                    manualTargetVideo = nil
                    if !num.isEmpty { Task { await manualScrape(target, number: num) } }
                }
            }
            Button("取消", role: .cancel) { manualTargetVideo = nil }
        } message: {
            Text(manualTargetVideo?.fileName ?? "")
        }
    }

    @ViewBuilder private var content: some View {
        if loading && currentCount == 0 {
            movieSkeletonGrid
        } else if currentCount == 0 {
            VStack(spacing: 16) {
                ContentUnavailableView(emptyTitle, systemImage: emptySymbol, description: Text(emptyDescription))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                LazyVStack(spacing: 14) {
                    switch mode {
                    case .movies:
                        if !hasActiveFilters, let hero = heroMovie {
                            heroBanner(hero)
                        }
                        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 14) {
                            ForEach(pageItems) { movie in
                                movieCell(movie)
                                    .onAppear { loadMoreIfNeeded(movie) }
                            }
                        }
                    case .pending:
                        if pendingFilter != .unrecognized && !issues.isEmpty {
                            HStack {
                                Label("刮削失败 (\(issueTotal))", systemImage: "exclamationmark.triangle.fill")
                                    .font(.caption.bold())
                                    .foregroundStyle(.orange)
                                Spacer()
                            }
                            ForEach(issues) { issue in
                                issueCell(issue).onAppear { loadMoreIfNeeded(issue) }
                            }
                        }
                        if pendingFilter != .failed && !unrecognized.isEmpty {
                            HStack {
                                Label("未识别视频 (\(unrecognizedTotal))", systemImage: "doc.questionmark.fill")
                                    .font(.caption.bold())
                                    .foregroundStyle(.purple)
                                Spacer()
                            }
                            .padding(.top, (pendingFilter == .all && !issues.isEmpty) ? 6 : 0)
                            ForEach(unrecognized) { item in
                                unrecognizedCell(item).onAppear { loadMoreIfNeeded(item) }
                            }
                        }
                    case .performers:
                        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                            ForEach(performers) { person in
                                performerCell(person).onAppear { loadMoreIfNeeded(person) }
                            }
                        }
                    }
                    if loadingMore { ProgressView().padding() }
                }
                .padding(14)
            }
        }
    }

    private func heroBanner(_ movie: MovieMetadata) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            ZStack(alignment: .topTrailing) {
                ZStack {
                    Color.black
                    CachedImageView(
                        url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) },
                        contentMode: .fit,
                        minHeight: 180,
                        maxPixel: 1000
                    )
                }
                .aspectRatio(3 / 2, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .onTapGesture { selectedMovie = movie }

                Button {
                    cycleHero()
                } label: {
                    Label("换一部", systemImage: "shuffle")
                        .font(.caption2.bold())
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(.ultraThinMaterial, in: Capsule())
                        .foregroundStyle(.white)
                }
                .padding(10)
            }

            HStack(spacing: 6) {
                Text(movie.number)
                    .font(.caption2.bold())
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3)
                    .background(LinearGradient(colors: [.purple, .pink], startPoint: .leading, endPoint: .trailing))
                    .foregroundStyle(.white)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                if let date = movie.releaseDate, !date.isEmpty {
                    Text(date).font(.caption2).foregroundStyle(.secondary)
                }
                if let maker = movie.maker, !maker.isEmpty {
                    Text(maker).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
            }

            Text(movie.title)
                .font(.subheadline.bold())
                .lineLimit(2)

            HStack(spacing: 10) {
                if let assetId = movie.assetId {
                    Button {
                        Task { await playAsset(assetId, title: movie.title) }
                    } label: {
                        Label("立即播放", systemImage: "play.fill")
                            .font(.caption.bold())
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 6)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(.purple)
                }
                Button {
                    selectedMovie = movie
                } label: {
                    Label("详情", systemImage: "info.circle")
                        .font(.caption.bold())
                        .padding(.vertical, 6)
                        .padding(.horizontal, 10)
                }
                .buttonStyle(.bordered)
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.22), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private var movieSkeletonGrid: some View {
        ScrollView {
            LazyVStack(spacing: 14) {
                if mode == .movies && !hasActiveFilters {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .fill(Color.secondary.opacity(0.14))
                        .frame(height: 176)
                }
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 14) {
                    ForEach(0..<8, id: \.self) { _ in
                        VStack(alignment: .leading, spacing: 8) {
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .fill(Color.secondary.opacity(0.16))
                                .aspectRatio(3 / 2, contentMode: .fit)
                            RoundedRectangle(cornerRadius: 4)
                                .fill(Color.secondary.opacity(0.14))
                                .frame(height: 12)
                            RoundedRectangle(cornerRadius: 4)
                                .fill(Color.secondary.opacity(0.10))
                                .frame(width: 90, height: 10)
                        }
                    }
                }
            }
            .padding(.horizontal)
            .padding(.top, 10)
            .redacted(reason: .placeholder)
        }
    }

    private var castRail: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("热门演员", systemImage: "star.fill")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
                Spacer()
                Button("全部演员") { mode = .performers }
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.purple)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 12) {
                    ForEach(topPerformers) { person in
                        let active = selectedActor == person.name
                        Button {
                            selectedActor = active ? "" : person.name
                            resetAndLoad()
                        } label: {
                            VStack(spacing: 5) {
                                ZStack(alignment: .bottomTrailing) {
                                    Group {
                                        if let raw = person.avatarUrl, !raw.isEmpty, let url = model.proxyScraperImageURL(rawUrl: raw) {
                                            CachedImageView(url: url, contentMode: .fill, minHeight: 54, maxPixel: 200)
                                        } else {
                                            ZStack {
                                                LinearGradient(colors: [.indigo, .purple], startPoint: .topLeading, endPoint: .bottomTrailing)
                                                Text(String(person.name.prefix(1))).font(.headline.bold()).foregroundStyle(.white)
                                            }
                                        }
                                    }
                                    .frame(width: 54, height: 54)
                                    .clipShape(Circle())
                                    .overlay(Circle().stroke(active ? Color.pink : Color.white.opacity(0.18), lineWidth: active ? 2.5 : 1))

                                    Text("\(person.movieCount)")
                                        .font(.system(size: 9, weight: .bold))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1)
                                        .background(.black.opacity(0.82), in: Capsule())
                                        .foregroundStyle(.white)
                                }
                                Text(person.name)
                                    .font(.system(size: 11, weight: active ? .bold : .regular))
                                    .foregroundStyle(active ? .pink : .secondary)
                                    .lineLimit(1)
                                    .frame(width: 62)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private var quickActionBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                let isFavOnly = mode == .movies ? onlyFavMovies : onlyFavPerformers
                let favCount = mode == .movies ? favs.movies.count : favs.performers.count
                Button {
                    if mode == .movies { onlyFavMovies.toggle() } else { onlyFavPerformers.toggle() }
                    resetAndLoad()
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: isFavOnly ? "heart.fill" : "heart")
                        Text("仅看收藏")
                        if favCount > 0 {
                            Text("(\(favCount))")
                        }
                    }
                    .font(.caption.weight(.semibold))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(isFavOnly ? Color.pink.opacity(0.22) : Color.secondary.opacity(0.12), in: Capsule())
                    .foregroundStyle(isFavOnly ? .pink : .primary)
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal)
        }
        .padding(.top, 8)
        .padding(.bottom, 2)
    }

    private var pendingSubFilterBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(PendingSubFilter.allCases) { sub in
                    let count: Int = {
                        switch sub {
                        case .all: return issueTotal + unrecognizedTotal
                        case .failed: return issueTotal
                        case .unrecognized: return unrecognizedTotal
                        }
                    }()
                    let active = pendingFilter == sub
                    Button {
                        pendingFilter = sub
                    } label: {
                        Text("\(sub.rawValue) (\(count))")
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(active ? Color.purple.opacity(0.22) : Color.secondary.opacity(0.12), in: Capsule())
                            .foregroundStyle(active ? .purple : .primary)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal)
        }
        .padding(.top, 8)
        .padding(.bottom, 2)
    }

    private var currentCount: Int {
        switch mode {
        case .movies: return pageItems.count
        case .pending:
            switch pendingFilter {
            case .all: return issues.count + unrecognized.count
            case .failed: return issues.count
            case .unrecognized: return unrecognized.count
            }
        case .performers: return performers.count
        }
    }
    private var emptyTitle: String {
        switch mode {
        case .movies: return onlyFavMovies ? "暂无收藏影片" : "暂无匹配的影视作品"
        case .pending:
            switch pendingFilter {
            case .all: return "暂无待处理项"
            case .failed: return "暂无刮削失败记录"
            case .unrecognized: return "暂无无法识别视频"
            }
        case .performers: return onlyFavPerformers ? "暂无收藏演员" : "暂无演员档案"
        }
    }
    private var emptySymbol: String {
        switch mode {
        case .movies: return onlyFavMovies ? "heart.slash" : "film.stack"
        case .pending: return "checkmark.shield"
        case .performers: return onlyFavPerformers ? "heart.slash" : "person.2"
        }
    }
    private var emptyDescription: String {
        switch mode {
        case .movies: return onlyFavMovies ? "点击影片卡片或详情页的红心即可加入收藏。" : "服务端仅返回已验证影视记录；切换到顶部「数据」页可增量同步与刮削。"
        case .pending: return "刮削失败与无法识别番号的视频会统一归集在这里处理。"
        case .performers: return onlyFavPerformers ? "点击演员卡片或详情页的红心即可加入收藏。" : "完成影片刮削后会自动入库演员档案。"
        }
    }

    private var sortLabel: String {
        switch sort {
        case "release_date_desc": return "发行最新"
        case "release_date_asc": return "发行最早"
        case "updated_at_desc": return "入库最新"
        case "updated_at_asc": return "入库最早"
        case "number": return "番号 A-Z"
        case "title": return "片名 A-Z"
        default: return "排序"
        }
    }

    private var movieFilters: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 12) {
                Menu {
                    Button("发行日期：最新优先") { sort = "release_date_desc" }
                    Button("发行日期：最早优先") { sort = "release_date_asc" }
                    Divider()
                    Button("刮削入库：最新优先") { sort = "updated_at_desc" }
                    Button("刮削入库：最早优先") { sort = "updated_at_asc" }
                    Divider()
                    Button("番号排序：A-Z") { sort = "number" }
                    Button("片名排序：A-Z") { sort = "title" }
                } label: {
                    Label(sortLabel, systemImage: "arrow.up.arrow.down")
                }

                Menu("片厂") {
                    Button("全部") { selectedMaker = ""; resetAndLoad() }
                    ForEach(makers, id: \.self) { value in Button(value) { selectedMaker = value; resetAndLoad() } }
                }
                Menu("演员") {
                    Button("全部") { selectedActor = ""; resetAndLoad() }
                    ForEach(actors, id: \.self) { value in Button(value) { selectedActor = value; resetAndLoad() } }
                }
                Spacer()
            }
            .font(.caption)
            .padding(.top, 8)

            if !selectedMaker.isEmpty || !selectedActor.isEmpty {
                HStack(spacing: 8) {
                    if !selectedMaker.isEmpty { Text("片厂: \(selectedMaker)") }
                    if !selectedActor.isEmpty { Text("演员: \(selectedActor)") }
                    Button("清除") { selectedMaker = ""; selectedActor = ""; resetAndLoad() }
                        .foregroundStyle(.purple)
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal)
        .onChange(of: sort) { _, _ in resetAndLoad() }
    }

    private var activeMovieFilterCount: Int {
        (onlyFavMovies ? 1 : 0) + (sort != "release_date_desc" ? 1 : 0) + (!selectedMaker.isEmpty ? 1 : 0) + (!selectedActor.isEmpty ? 1 : 0)
    }

    private var activePerformerFilterCount: Int {
        (onlyFavPerformers ? 1 : 0)
            + (performerSort != "latest_release" ? 1 : 0)
            + (!performerAgeRange.isEmpty ? 1 : 0)
            + (!performerHeightRange.isEmpty ? 1 : 0)
            + (!performerCup.isEmpty ? 1 : 0)
            + (performerMinMovies > 0 ? 1 : 0)
            + (!performerProfileFilter.isEmpty ? 1 : 0)
    }

    private var activeMovieFilterStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                if onlyFavMovies {
                    activeTagChip("❤️ 仅看收藏") { onlyFavMovies = false; resetAndLoad() }
                }
                if sort != "release_date_desc" {
                    activeTagChip(sortLabel) { sort = "release_date_desc"; resetAndLoad() }
                }
                if !selectedMaker.isEmpty {
                    activeTagChip("片厂: \(selectedMaker)") { selectedMaker = ""; resetAndLoad() }
                }
                if !selectedActor.isEmpty {
                    activeTagChip("演员: \(selectedActor)") { selectedActor = ""; resetAndLoad() }
                }
                Button("清空") {
                    onlyFavMovies = false
                    sort = "release_date_desc"
                    selectedMaker = ""
                    selectedActor = ""
                    resetAndLoad()
                }
                .font(.caption2.bold())
                .foregroundStyle(.red)
            }
            .padding(.horizontal)
        }
        .padding(.vertical, 6)
    }

    private var activePerformerFilterStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                if onlyFavPerformers {
                    activeTagChip("❤️ 仅看收藏") { onlyFavPerformers = false; resetAndLoad() }
                }
                if performerSort != "latest_release" {
                    activeTagChip(performerSortLabel) { performerSort = "latest_release"; resetAndLoad() }
                }
                if !performerAgeRange.isEmpty {
                    activeTagChip("年龄: \(performerAgeRange)") { performerAgeRange = ""; resetAndLoad() }
                }
                if !performerHeightRange.isEmpty {
                    activeTagChip("身高: \(performerHeightRange)") { performerHeightRange = ""; resetAndLoad() }
                }
                if !performerCup.isEmpty {
                    activeTagChip("\(performerCup)罩杯") { performerCup = ""; resetAndLoad() }
                }
                if performerMinMovies > 0 {
                    activeTagChip("≥\(performerMinMovies)部") { performerMinMovies = 0; resetAndLoad() }
                }
                if !performerProfileFilter.isEmpty {
                    activeTagChip(performerProfileFilter == "has_avatar" ? "已有头像" : "待补全") { performerProfileFilter = ""; resetAndLoad() }
                }
                Button("清空") {
                    onlyFavPerformers = false
                    performerSort = "latest_release"
                    performerAgeRange = ""
                    performerHeightRange = ""
                    performerCup = ""
                    performerMinMovies = 0
                    performerProfileFilter = ""
                    resetAndLoad()
                }
                .font(.caption2.bold())
                .foregroundStyle(.red)
            }
            .padding(.horizontal)
        }
        .padding(.vertical, 6)
    }

    private func activeTagChip(_ title: String, onClear: @escaping () -> Void) -> some View {
        HStack(spacing: 4) {
            Text(title)
            Button(action: onClear) {
                Image(systemName: "xmark.circle.fill")
                    .font(.caption2)
            }
            .buttonStyle(.plain)
        }
        .font(.caption2.weight(.semibold))
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .background(Color.purple.opacity(0.18), in: Capsule())
        .foregroundStyle(.purple)
    }

    private func drawerChoiceChip(_ title: String, active: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.caption.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 9)
                .padding(.horizontal, 6)
                .background(active ? Color.purple : Color.secondary.opacity(0.14), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                .foregroundStyle(active ? .white : .primary)
        }
        .buttonStyle(.plain)
    }

    private var filterDrawerSheet: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if mode == .movies {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("收藏筛选").font(.footnote.bold()).foregroundStyle(.secondary)
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                drawerChoiceChip("全部影片", active: !onlyFavMovies) {
                                    if onlyFavMovies { onlyFavMovies = false; resetAndLoad() }
                                }
                                drawerChoiceChip("❤️ 仅看收藏\(favs.movies.isEmpty ? "" : " (\(favs.movies.count))")", active: onlyFavMovies) {
                                    if !onlyFavMovies { onlyFavMovies = true; resetAndLoad() }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("排序方式").font(.footnote.bold()).foregroundStyle(.secondary)
                            let opts: [(String, String)] = [
                                ("release_date_desc", "发行日期：最新"),
                                ("release_date_asc", "发行日期：最早"),
                                ("updated_at_desc", "入库时间：最新"),
                                ("updated_at_asc", "入库时间：最早"),
                                ("number", "番号：A - Z"),
                                ("title", "片名：A - Z"),
                            ]
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(opts, id: \.0) { val, label in
                                    drawerChoiceChip(label, active: sort == val) {
                                        sort = val
                                        resetAndLoad()
                                    }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("片厂筛选").font(.footnote.bold()).foregroundStyle(.secondary)
                            Picker("片厂", selection: Binding(
                                get: { selectedMaker },
                                set: { selectedMaker = $0; resetAndLoad() }
                            )) {
                                Text("全部片厂").tag("")
                                ForEach(makers, id: \.self) { m in Text(m).tag(m) }
                            }
                            .pickerStyle(.menu)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("演员筛选").font(.footnote.bold()).foregroundStyle(.secondary)
                            Picker("演员", selection: Binding(
                                get: { selectedActor },
                                set: { selectedActor = $0; resetAndLoad() }
                            )) {
                                Text("全部演员").tag("")
                                ForEach(actors, id: \.self) { a in Text(a).tag(a) }
                            }
                            .pickerStyle(.menu)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
                        }
                    } else {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("收藏筛选").font(.footnote.bold()).foregroundStyle(.secondary)
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                drawerChoiceChip("全部演员", active: !onlyFavPerformers) {
                                    if onlyFavPerformers { onlyFavPerformers = false; resetAndLoad() }
                                }
                                drawerChoiceChip("❤️ 仅看收藏\(favs.performers.isEmpty ? "" : " (\(favs.performers.count))")", active: onlyFavPerformers) {
                                    if !onlyFavPerformers { onlyFavPerformers = true; resetAndLoad() }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("排序方式").font(.footnote.bold()).foregroundStyle(.secondary)
                            let opts: [(String, String)] = [
                                ("latest_release", "最新作品时间（默认）"),
                                ("latest_added", "最新入库时间"),
                                ("age_asc", "年龄：年轻优先"),
                                ("age_desc", "年龄：年长优先"),
                                ("movie_count_desc", "作品数：多 → 少"),
                                ("movie_count_asc", "作品数：少 → 多"),
                            ]
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(opts, id: \.0) { val, label in
                                    drawerChoiceChip(label, active: performerSort == val) {
                                        performerSort = val
                                        resetAndLoad()
                                    }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("年龄段").font(.footnote.bold()).foregroundStyle(.secondary)
                            let ages: [(String, String)] = [("", "全部"), ("under22", "<22岁"), ("22_25", "22-25岁"), ("26_30", "26-30岁"), ("over30", "30岁+")]
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(ages, id: \.0) { val, label in
                                    drawerChoiceChip(label, active: performerAgeRange == val) {
                                        performerAgeRange = val
                                        resetAndLoad()
                                    }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("身高").font(.footnote.bold()).foregroundStyle(.secondary)
                            let heights: [(String, String)] = [("", "全部"), ("under155", "<155cm"), ("155_165", "155-165cm"), ("over165", ">165cm")]
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(heights, id: \.0) { val, label in
                                    drawerChoiceChip(label, active: performerHeightRange == val) {
                                        performerHeightRange = val
                                        resetAndLoad()
                                    }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("罩杯").font(.footnote.bold()).foregroundStyle(.secondary)
                            let cups = ["", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J"]
                            LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 5), spacing: 8) {
                                ForEach(cups, id: \.self) { c in
                                    drawerChoiceChip(c.isEmpty ? "全部" : c, active: performerCup == c) {
                                        performerCup = c
                                        resetAndLoad()
                                    }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("作品数门槛").font(.footnote.bold()).foregroundStyle(.secondary)
                            let mins: [(Int, String)] = [(0, "全部"), (3, "≥3部"), (5, "≥5部"), (10, "≥10部"), (20, "≥20部")]
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(mins, id: \.0) { val, label in
                                    drawerChoiceChip(label, active: performerMinMovies == val) {
                                        performerMinMovies = val
                                        resetAndLoad()
                                    }
                                }
                            }
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text("档案状态").font(.footnote.bold()).foregroundStyle(.secondary)
                            let profiles: [(String, String)] = [("", "全部"), ("has_avatar", "已有头像"), ("missing_profile", "待补全")]
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(profiles, id: \.0) { val, label in
                                    drawerChoiceChip(label, active: performerProfileFilter == val) {
                                        performerProfileFilter = val
                                        resetAndLoad()
                                    }
                                }
                            }
                        }
                    }
                }
                .padding(16)
            }
            .navigationTitle(mode == .movies ? "影视筛选与排序" : "演员筛选与排序")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("重置") {
                        if mode == .movies {
                            onlyFavMovies = false
                            sort = "release_date_desc"
                            selectedMaker = ""
                            selectedActor = ""
                        } else {
                            onlyFavPerformers = false
                            performerSort = "latest_release"
                            performerAgeRange = ""
                            performerHeightRange = ""
                            performerCup = ""
                            performerMinMovies = 0
                            performerProfileFilter = ""
                        }
                        resetAndLoad()
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { showFilterDrawer = false }
                        .bold()
                }
            }
        }
    }

    private var issueFilters: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack {
                issueChip("全部", value: "")
                ForEach(issueCounts) { item in issueChip("\(item.category) (\(item.count))", value: item.category) }
            }.padding(.horizontal)
        }.padding(.vertical, 6)
    }

    private var performerSortLabel: String {
        switch performerSort {
        case "latest_added": return "最新入库"
        case "age_asc": return "年龄升序"
        case "age_desc": return "年龄降序"
        case "movie_count_desc": return "作品最多"
        case "movie_count_asc": return "作品最少"
        default: return "最新作品"
        }
    }

    private var performerFilters: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 10) {
                Menu {
                    Button("最新作品时间（默认）") { performerSort = "latest_release"; resetAndLoad() }
                    Button("最新入库时间") { performerSort = "latest_added"; resetAndLoad() }
                    Button("年龄：年轻优先") { performerSort = "age_asc"; resetAndLoad() }
                    Button("年龄：年长优先") { performerSort = "age_desc"; resetAndLoad() }
                    Button("作品数：多 → 少") { performerSort = "movie_count_desc"; resetAndLoad() }
                    Button("作品数：少 → 多") { performerSort = "movie_count_asc"; resetAndLoad() }
                } label: {
                    Label(performerSortLabel, systemImage: "arrow.up.arrow.down")
                }

                Menu {
                    Button("全部年龄") { performerAgeRange = ""; resetAndLoad() }
                    Button("22岁以下") { performerAgeRange = "under22"; resetAndLoad() }
                    Button("22 - 25岁") { performerAgeRange = "22_25"; resetAndLoad() }
                    Button("26 - 30岁") { performerAgeRange = "26_30"; resetAndLoad() }
                    Button("30岁以上") { performerAgeRange = "over30"; resetAndLoad() }
                } label: {
                    Label(performerAgeRange.isEmpty ? "年龄" : performerAgeRange, systemImage: "calendar")
                }

                Menu {
                    Button("全部身高") { performerHeightRange = ""; resetAndLoad() }
                    Button("155cm以下") { performerHeightRange = "under155"; resetAndLoad() }
                    Button("155 - 165cm") { performerHeightRange = "155_165"; resetAndLoad() }
                    Button("165cm以上") { performerHeightRange = "over165"; resetAndLoad() }
                } label: {
                    Label(performerHeightRange.isEmpty ? "身高" : performerHeightRange, systemImage: "ruler")
                }

                Menu {
                    Button("全部罩杯") { performerCup = ""; resetAndLoad() }
                    ForEach(["A", "B", "C", "D", "E", "F", "G", "H", "I", "J"], id: \.self) { c in
                        Button("\(c) 罩杯") { performerCup = c; resetAndLoad() }
                    }
                } label: {
                    Label(performerCup.isEmpty ? "罩杯" : "\(performerCup)杯", systemImage: "sparkles")
                }

                Menu {
                    Button("全部作品数") { performerMinMovies = 0; resetAndLoad() }
                    Button("≥ 3 部") { performerMinMovies = 3; resetAndLoad() }
                    Button("≥ 5 部") { performerMinMovies = 5; resetAndLoad() }
                    Button("≥ 10 部") { performerMinMovies = 10; resetAndLoad() }
                    Button("≥ 20 部") { performerMinMovies = 20; resetAndLoad() }
                } label: {
                    Label(performerMinMovies > 0 ? "≥\(performerMinMovies)部" : "作品数", systemImage: "film")
                }

                if !performerAgeRange.isEmpty || !performerHeightRange.isEmpty || !performerCup.isEmpty || performerMinMovies > 0 || performerSort != "latest_release" {
                    Button("重置") {
                        performerSort = "latest_release"
                        performerAgeRange = ""
                        performerHeightRange = ""
                        performerCup = ""
                        performerMinMovies = 0
                        performerProfileFilter = ""
                        resetAndLoad()
                    }
                    .foregroundStyle(.purple)
                }
            }
            .font(.caption)
            .padding(.horizontal)
        }
        .padding(.vertical, 6)
    }
    private func issueChip(_ title: String, value: String) -> some View {
        Button(title) { issueCategory = value; resetAndLoad() }
            .font(.caption).padding(.horizontal, 10).padding(.vertical, 6)
            .background(issueCategory == value ? Color.purple.opacity(0.2) : Color.secondary.opacity(0.12), in: Capsule())
    }

    private func movieCell(_ movie: MovieMetadata) -> some View {
        let isFav = favs.isMovieFav(movie.number)
        return ZStack(alignment: .topTrailing) {
            VStack(alignment: .leading, spacing: 5) {
                ZStack {
                    Color.black.opacity(0.88)
                    CachedImageView(url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fit, minHeight: 108, maxPixel: 720)
                }
                .overlay(alignment: .topLeading) {
                    Text(movie.number)
                        .font(.system(size: 9, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.black.opacity(0.72), in: RoundedRectangle(cornerRadius: 4))
                        .foregroundStyle(.white)
                        .padding(6)
                }
                .aspectRatio(3 / 2, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

                Text(movie.title)
                    .font(.caption.weight(.semibold))
                    .lineLimit(2)
                HStack(spacing: 4) {
                    Text(movie.actors.prefix(2).joined(separator: " · ").isEmpty ? (movie.maker ?? "未知") : movie.actors.prefix(2).joined(separator: " · "))
                        .lineLimit(1)
                    Spacer(minLength: 2)
                    if let date = movie.releaseDate, !date.isEmpty {
                        Text(date)
                    }
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
            .contentShape(Rectangle())
            .onTapGesture { selectedMovie = movie }

            if isFav {
                Image(systemName: "heart.fill")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.pink)
                    .frame(width: 24, height: 24)
                    .background(.black.opacity(0.6), in: Circle())
                    .padding(6)
                    .allowsHitTesting(false)
            }
        }
    }

    private func issueCell(_ issue: ScrapeIssue) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(issue.number).font(.subheadline.bold())
                Spacer()
                Text(issue.category)
                    .font(.caption2.bold())
                    .padding(.horizontal, 7)
                    .padding(.vertical, 2)
                    .background((issue.retryable ? Color.orange : Color.red).opacity(0.16), in: Capsule())
                    .foregroundStyle(issue.retryable ? .orange : .red)
            }
            Text(issue.error).font(.caption).foregroundStyle(.secondary).lineLimit(3)
            if let files = issue.files, !files.isEmpty {
                ForEach(files) { f in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(f.fileName).font(.caption2.weight(.semibold)).lineLimit(1)
                        Text(f.remotePath).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(2)
                    }
                    .padding(7)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(.black.opacity(0.14), in: RoundedRectangle(cornerRadius: 6))
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
    }

    private func unrecognizedCell(_ item: UnrecognizedVideo) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(item.fileName).font(.subheadline.weight(.semibold))
            Text(item.remotePath).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            HStack(spacing: 10) {
                Button {
                    manualNumberInput = ""
                    manualTargetVideo = item
                } label: {
                    Label("指定番号", systemImage: "tag")
                        .font(.caption2.weight(.semibold))
                }
                .buttonStyle(.bordered)
                .tint(.purple)

                Button(role: .destructive) {
                    Task { await ignoreVideo(item) }
                } label: {
                    Label("忽略", systemImage: "eye.slash")
                        .font(.caption2.weight(.semibold))
                }
                .buttonStyle(.bordered)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
    }

    private func performerCell(_ person: MoviePerformer) -> some View {
        let isFav = favs.isPerformerFav(person.id)
        return ZStack(alignment: .topTrailing) {
            VStack(alignment: .leading, spacing: 6) {
                ZStack(alignment: .bottomTrailing) {
                    Color.black.opacity(0.85)
                        .aspectRatio(1, contentMode: .fit)
                        .overlay(
                            Group {
                                if let raw = person.avatarUrl, !raw.isEmpty, let url = model.proxyScraperImageURL(rawUrl: raw) {
                                    CachedImageView(url: url, contentMode: .fill, minHeight: 110, maxPixel: 360)
                                } else {
                                    ZStack {
                                        LinearGradient(colors: [.indigo, .purple], startPoint: .topLeading, endPoint: .bottomTrailing)
                                        Text(String(person.name.prefix(1)))
                                            .font(.title2.bold())
                                            .foregroundStyle(.white)
                                    }
                                }
                            }
                        )
                        .clipped()

                    Text("\(person.movieCount) 部")
                        .font(.system(size: 10, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.black.opacity(0.75), in: Capsule())
                        .foregroundStyle(.white)
                        .padding(6)
                }
                .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

                VStack(alignment: .leading, spacing: 2) {
                    Text(person.name)
                        .font(.caption.bold())
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if let d = person.latestReleaseDate, !d.isEmpty {
                        Text(d)
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    } else if let h = person.heightCm, h > 0 {
                        Text("\(h)cm")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    } else {
                        Text("\(person.movieCount) 部作品")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                .padding(.horizontal, 2)
            }
            .contentShape(Rectangle())
            .onTapGesture { selectedPerformer = person }

            if isFav {
                Image(systemName: "heart.fill")
                    .font(.caption2.weight(.bold))
                    .foregroundStyle(.pink)
                    .frame(width: 24, height: 24)
                    .background(.black.opacity(0.6), in: Circle())
                    .padding(6)
                    .allowsHitTesting(false)
            }
        }
    }

    private func cycleHero() {
        guard randomHeroPool.count > 1 else {
            Task { await loadDiscoveryData() }
            return
        }
        var next = Int.random(in: 0..<randomHeroPool.count)
        if next == heroIndex % randomHeroPool.count {
            next = (next + 1) % randomHeroPool.count
        }
        heroIndex = next
    }

    private func loadDiscoveryData() async {
        guard let client = model.apiClient else { return }
        async let featured = try? client.getRandomFeaturedMovies(limit: 20)
        async let cast = try? client.listMoviePerformers(query: "", limit: 30, offset: 0)
        if let list = await featured, !list.isEmpty {
            randomHeroPool = list
            heroIndex = Int.random(in: 0..<list.count)
        }
        if let res = await cast {
            topPerformers = res.items
        }
    }

    private func requestMore() {
        guard !loadingMore else { return }
        loadingMore = true
        Task { await loadPage(reset: false) }
    }
    private func loadMoreIfNeeded(_ movie: MovieMetadata) {
        guard let index = pageItems.firstIndex(of: movie), index >= pageItems.count - 5, pageItems.count < total else { return }
        requestMore()
    }
    private func loadMoreIfNeeded(_ issue: ScrapeIssue) {
        guard let index = issues.firstIndex(of: issue), index >= issues.count - 5, issues.count < issueTotal else { return }
        requestMore()
    }
    private func loadMoreIfNeeded(_ item: UnrecognizedVideo) {
        guard let index = unrecognized.firstIndex(of: item), index >= unrecognized.count - 5, unrecognized.count < unrecognizedTotal else { return }
        requestMore()
    }
    private func loadMoreIfNeeded(_ person: MoviePerformer) {
        guard let index = performers.firstIndex(of: person), index >= performers.count - 5, performers.count < performerTotal else { return }
        requestMore()
    }
    private func resetAndLoad() { Task { await reload() } }
    private func reload() async {
        requestVersion += 1
        offset = 0; issueOffset = 0; unrecognizedOffset = 0
        pageItems = []; issues = []; unrecognized = []; performers = []; errorMessage = nil
        loading = true; loadingMore = false
        await loadPage(reset: true)
    }
    private func loadPage(reset: Bool) async {
        guard let client = model.apiClient else { errorMessage = "未连接到数据源"; return }
        let version = requestVersion
        if reset { loading = true } else { loadingMore = true }
        defer { if version == requestVersion { loading = false; loadingMore = false } }
        do {
            switch mode {
            case .movies:
                let result = try await client.queryMovieLibrary(query: searchQuery, maker: selectedMaker, actor: selectedActor, sort: sort, onlyFavorites: onlyFavMovies, limit: pageSize, offset: offset)
                guard version == requestVersion else { return }
                pageItems = reset ? result.items : pageItems + result.items; total = result.total; makers = result.makers; actors = result.actors; offset += result.items.count
            case .pending:
                if reset {
                    async let issueReq = client.listScrapeIssues(category: issueCategory, query: searchQuery, limit: pageSize, offset: 0)
                    async let unrecReq = client.listUnrecognizedVideos(limit: pageSize, offset: 0)
                    let (issueRes, unrecRes) = try await (issueReq, unrecReq)
                    guard version == requestVersion else { return }
                    issues = issueRes.items; issueTotal = issueRes.total; issueCounts = issueRes.counts; issueOffset = issueRes.items.count
                    unrecognized = unrecRes.items; unrecognizedTotal = unrecRes.total; unrecognizedOffset = unrecRes.items.count
                } else {
                    if issues.count < issueTotal {
                        let issueRes = try await client.listScrapeIssues(category: issueCategory, query: searchQuery, limit: pageSize, offset: issueOffset)
                        guard version == requestVersion else { return }
                        issues += issueRes.items; issueTotal = issueRes.total; issueOffset += issueRes.items.count
                    }
                    if unrecognized.count < unrecognizedTotal {
                        let unrecRes = try await client.listUnrecognizedVideos(limit: pageSize, offset: unrecognizedOffset)
                        guard version == requestVersion else { return }
                        unrecognized += unrecRes.items; unrecognizedTotal = unrecRes.total; unrecognizedOffset += unrecRes.items.count
                    }
                }
            case .performers:
                let result = try await client.listMoviePerformers(
                    query: searchQuery,
                    sort: performerSort,
                    minMovies: performerMinMovies > 0 ? performerMinMovies : nil,
                    ageRange: performerAgeRange,
                    heightRange: performerHeightRange,
                    cup: performerCup,
                    profileFilter: performerProfileFilter,
                    onlyFavorites: onlyFavPerformers,
                    limit: pageSize,
                    offset: offset
                )
                guard version == requestVersion else { return }
                performers = reset ? result.items : performers + result.items; performerTotal = result.total; offset += result.items.count
            }
        } catch { if version == requestVersion { errorMessage = error.localizedDescription } }
    }

    private func manualScrape(_ item: UnrecognizedVideo, number: String) async {
        guard let client = model.apiClient else { return }
        do {
            _ = try await client.scrapeAsset(assetId: item.id, fileName: item.fileName, remotePath: item.remotePath, number: number, force: true)
            await reload()
        } catch {
            errorMessage = "指定番号刮削失败：\(error.localizedDescription)"
        }
    }

    private func ignoreVideo(_ item: UnrecognizedVideo) async {
        guard let client = model.apiClient else { return }
        do {
            try await client.ignoreUnrecognizedVideo(assetId: item.id)
            unrecognized.removeAll { $0.id == item.id }
            unrecognizedTotal = max(0, unrecognizedTotal - 1)
        } catch {
            errorMessage = "忽略失败：\(error.localizedDescription)"
        }
    }

    private func playAsset(_ assetId: String, title: String) async {
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else {
            errorMessage = model.playbackError ?? "无法解析视频直链"
            return
        }
        preview = PreviewItem(url: source.url, isVideo: true, assetId: assetId, title: title, fallbackURL: source.fallbackURL, sourceLabel: source.label, remotePath: nil)
    }

    private var dataConsoleView: some View {
        let unconsumedAdded = (pipelineDiff?.consumed == true) ? 0 : (pipelineDiff?.addedCount ?? 0)
        let pureNewCount = batchStats?.newToScrape ?? max(0, (batchStats?.remainingToScrape ?? 0) - (batchStats?.failedCount ?? 0))
        let failedCount = batchStats?.failedCount ?? 0
        let unrecCount = batchStats?.unrecognizedVideos ?? unrecognizedTotal
        let totalIssuesCount = issueTotal + unrecognizedTotal

        return List {
            if let errorMessage {
                Section {
                    Label(errorMessage, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
            }

            // 顶部 5 项 KPI 汇总 + 一键增量入库（与 Web 端布局完全一致）
            Section {
                HStack(spacing: 0) {
                    kpiCell(value: "\(batchStats?.totalWhitelistedVideos ?? 0)", label: "库内视频", color: .primary)
                    kpiCell(value: "\(batchStats?.alreadyScraped ?? 0)", label: "已入库", color: .green)
                    kpiCell(value: "\(pureNewCount)", label: "待刮新片", color: .purple)
                    kpiCell(value: "\(failedCount)", label: "失败待重试", color: .orange)
                    kpiCell(value: "\(unrecCount)", label: "未识别", color: .secondary)
                }
                .padding(.vertical, 2)

                Button {
                    Task { await runOneClickIncremental() }
                } label: {
                    HStack {
                        Spacer()
                        if autoFlowRunning {
                            ProgressView().controlSize(.small).padding(.trailing, 4)
                            Text("流水线执行中…").font(.subheadline.weight(.semibold))
                        } else {
                            Label("一键增量入库", systemImage: "bolt.fill")
                                .font(.subheadline.weight(.semibold))
                        }
                        Spacer()
                    }
                    .padding(.vertical, 4)
                }
                .buttonStyle(.borderedProminent)
                .tint(.purple)
                .disabled(autoFlowRunning || pipelineSyncMode != nil || pipelineSyncProgress?.running == true || batchProgress?.running == true || pipelineExtracting)

                if let note = autoFlowNote, !note.isEmpty {
                    Text(note)
                        .font(.caption)
                        .foregroundStyle(.purple)
                }
            }

            // 阶段 ①：网盘同步 & 变动识别
            Section {
                if let sp = pipelineSyncProgress, sp.running || sp.phase == "done" {
                    let totalDirs = max(sp.discoveredDirs, sp.scannedDirs, 1)
                    let pct: Double = sp.phase == "done" ? 1.0 : sp.phase == "reconciling" ? 0.97 : sp.phase == "writing" ? 0.92 : min(0.9, Double(sp.scannedDirs) / Double(totalDirs) * 0.9)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(sp.running ? "同步中（\(Int(pct * 100))%）：已扫目录 \(sp.scannedDirs)/\(totalDirs) · 发现文件 \(sp.foundFiles)\(sp.skippedDirs > 0 ? " · 跳过 \(sp.skippedDirs)" : "")" : "同步已完成：扫描 \(sp.scannedDirs) 目录 · 发现 \(sp.foundFiles) 文件")
                            .font(.caption2)
                        ProgressView(value: pct, total: 1.0)
                        if let cur = sp.currentDir, !cur.isEmpty {
                            Text(cur).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
                HStack(spacing: 12) {
                    Button {
                        Task { await runPipelineDbSync(mode: "incremental") }
                    } label: {
                        Label(pipelineSyncMode == "incremental" ? "增量更新中…" : "增量更新", systemImage: "bolt.fill")
                            .font(.subheadline.weight(.semibold))
                    }
                    .disabled(pipelineSyncMode != nil || pipelineSyncProgress?.running == true || batchProgress?.running == true)
                    .tint(.purple)

                    Spacer()

                    Button {
                        Task { await runPipelineDbSync(mode: "full") }
                    } label: {
                        Label(pipelineSyncMode == "full" ? "全量更新中…" : "全量更新", systemImage: "arrow.triangle.2.circlepath")
                            .font(.subheadline)
                    }
                    .disabled(pipelineSyncMode != nil || pipelineSyncProgress?.running == true || batchProgress?.running == true)
                }

                if let diff = pipelineDiff, let runId = diff.runId, !runId.isEmpty {
                    HStack {
                        Text(diff.addedCount > 0
                             ? "本次\(diff.mode == "full" ? "全量" : "增量") · 获取 +\(diff.addedCount) 个新视频 · 删除 -\(diff.removedCount)"
                             : "本次\(diff.mode == "full" ? "全量" : "增量") · 无新增视频 · 删除 -\(diff.removedCount)")
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(diff.addedCount > 0 ? .primary : .secondary)
                        Spacer()
                        Text(unconsumedAdded > 0 ? "待提取 +\(unconsumedAdded)" : "已消化 ✓")
                            .font(.caption2.bold())
                            .foregroundStyle(unconsumedAdded > 0 ? .purple : .green)
                    }
                    if let sum = diff.consumedSummary, diff.consumed == true, !sum.isEmpty {
                        Text("✓ \(sum)")
                            .font(.caption2)
                            .foregroundStyle(.green)
                    }
                    if diff.addedCount > 0 {
                        DisclosureGroup("新增视频（\(diff.addedCount)）", isExpanded: $showDiffAdded) {
                            ForEach(diff.added.prefix(30)) { item in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(item.fileName).font(.caption).lineLimit(1)
                                    Text(item.remotePath).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                                }
                            }
                        }
                        .font(.caption)
                    }
                    if diff.removedCount > 0 {
                        DisclosureGroup("删除/改名视频（\(diff.removedCount)）", isExpanded: $showDiffRemoved) {
                            ForEach(diff.removed.prefix(30)) { item in
                                Text(item.fileName).font(.caption).foregroundStyle(.red).lineLimit(1)
                            }
                        }
                        .font(.caption)
                    }
                }
            } header: {
                Text("① 网盘同步 & 变动识别")
            }

            // 阶段 ②：番号提取 & 文件绑定
            Section {
                HStack(spacing: 12) {
                    Button(pipelineExtracting ? "提取中…" : (unconsumedAdded > 0 ? "提取本批新增（\(unconsumedAdded)）" : "核对新增")) {
                        Task { await runPipelineExtract(scope: "new") }
                    }
                    .disabled(pipelineExtracting)
                    .tint(.purple)

                    Spacer()

                    Button("重跑未识别") {
                        Task { await runPipelineExtract(scope: "unrecognized") }
                    }
                    .disabled(pipelineExtracting)
                }
                if let ex = pipelineExtract {
                    Text("扫描 \(ex.scannedVideos) 视频 · 识别 \(ex.extractedNumbers) 番号（已在库 \(ex.alreadyInLibrary)）· 移交待刮 \(ex.pendingQueueCount)")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    if !ex.pendingNumbersPreview.isEmpty {
                        Text("待刮削预览：\(ex.pendingNumbersPreview.prefix(12).joined(separator: "、"))\(ex.pendingQueueCount > 12 ? " 等共 \(ex.pendingQueueCount) 部" : "")")
                            .font(.caption2)
                            .foregroundStyle(.purple)
                    }
                }
            } header: {
                Text("② 番号提取 & 文件绑定")
            }

            // 阶段 ③：刮削入库 & 演员补全
            Section {
                if let progress = batchProgress, progress.running {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("当前：\(progress.currentNumber ?? "准备中…") (\(progress.completed)/\(progress.total) · 剩余 \(max(0, progress.total - progress.completed)))")
                            .font(.caption2)
                        if let actorStatus = progress.currentActorStatus, !actorStatus.isEmpty {
                            Text(actorStatus)
                                .font(.caption2)
                                .foregroundStyle(.purple)
                        }
                        ProgressView(value: Double(progress.completed), total: Double(max(progress.total, 1)))
                    }
                    Button("停止刮削", role: .destructive) { Task { await stopScrape() } }
                } else {
                    Button {
                        Task { await startManualScrape(mode: "new") }
                    } label: {
                        Label("刮削新片队列（\(pureNewCount)）", systemImage: "play.fill")
                            .font(.body.weight(.semibold))
                    }
                    .tint(.purple)
                    .disabled(pureNewCount == 0)

                    if failedCount > 0 {
                        Button("重试失败项（\(failedCount)）") {
                            Task { await startManualScrape(mode: "failed") }
                        }
                    }
                }

                if let recent = batchProgress?.recentItems, !recent.isEmpty {
                    DisclosureGroup("本次刮削回执（\(recent.count)）") {
                        ForEach(recent.prefix(20)) { item in
                            HStack(spacing: 6) {
                                Image(systemName: item.ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                                    .foregroundStyle(item.ok ? .green : .red)
                                    .font(.caption)
                                Text(item.number)
                                    .font(.caption.bold())
                                Text(item.ok ? (item.title ?? "") : (item.error ?? "失败"))
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                        }
                    }
                    .font(.caption)
                }

                let missingActors = enrichStats?.missingCount ?? 0
                if let ep = enrichProgress, ep.running {
                    VStack(alignment: .leading, spacing: 6) {
                        Text("补全演员中：\(ep.currentName ?? "扫描中…") (\(ep.completed)/\(ep.total))")
                            .font(.caption2)
                        ProgressView(value: Double(ep.completed), total: Double(max(ep.total, 1)))
                    }
                    Button("停止演员补全", role: .destructive) { Task { await stopEnrich() } }
                } else if missingActors > 0 {
                    Button("独立补全缺失演员资料（\(missingActors)）") {
                        Task { await startEnrich(forceAll: false) }
                    }
                    .font(.caption)
                }
            } header: {
                Text("③ 刮削入库 & 演员补全")
            }

            // 底部默认折叠的异常记录明细（与 Web 端一致）
            Section {
                DisclosureGroup(isExpanded: $showIssuesSection) {
                    VStack(alignment: .leading, spacing: 10) {
                        pendingSubFilterBar
                        if pendingFilter != .unrecognized && !issueCounts.isEmpty {
                            issueFilters
                        }
                        if pendingFilter != .unrecognized && !issues.isEmpty {
                            ForEach(issues) { issue in
                                issueCell(issue).onAppear { loadMoreIfNeeded(issue) }
                            }
                        }
                        if pendingFilter != .failed && !unrecognized.isEmpty {
                            ForEach(unrecognized) { item in
                                unrecognizedCell(item).onAppear { loadMoreIfNeeded(item) }
                            }
                        }
                        if (pendingFilter == .all && issues.isEmpty && unrecognized.isEmpty)
                            || (pendingFilter == .failed && issues.isEmpty)
                            || (pendingFilter == .unrecognized && unrecognized.isEmpty) {
                            Text("暂无异常记录")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .padding(.vertical, 6)
                        }
                    }
                    .padding(.top, 4)
                } label: {
                    HStack {
                        Label("异常记录明细", systemImage: "exclamationmark.triangle")
                            .font(.subheadline.weight(.semibold))
                        Spacer()
                        Text("失败 \(issueTotal) · 未识别 \(unrecognizedTotal)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
            } footer: {
                if totalIssuesCount > 0 {
                    Text("历史刮削失败与无法识别番号的视频默认折叠收纳，展开可逐条重试或手动指定番号。")
                }
            }
        }
        .listStyle(.insetGrouped)
        .task { await refreshConsoleState() }
    }

    private func kpiCell(value: String, label: String, color: Color) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(.headline.monospacedDigit())
                .foregroundStyle(color)
            Text(label)
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
    }

    private func runOneClickIncremental() async {
        guard let c = model.apiClient, !autoFlowRunning else { return }
        autoFlowRunning = true
        autoFlowNote = "阶段 ①/③：正在增量同步网盘目录…"
        defer { autoFlowRunning = false }
        do {
            var p = try await c.runMovieDbSync(mode: "incremental")
            pipelineSyncProgress = p
            while p.running && !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(1200))
                p = try await c.getMovieDbSyncProgress()
                pipelineSyncProgress = p
            }
            if let err = p.error, !err.isEmpty {
                errorMessage = err
                autoFlowNote = nil
                return
            }
            autoFlowNote = "阶段 ②/③：正在提取新增番号并补齐绑定…"
            let ex = try await c.extractMovieNumbers(scope: "new")
            pipelineExtract = ex
            await refreshConsoleState()
            if ex.pendingQueueCount > 0 {
                autoFlowNote = "阶段 ③/③：发现 \(ex.pendingQueueCount) 部新片，已启动实时刮削…"
                await startManualScrape(mode: "new")
            } else {
                autoFlowNote = ex.alreadyInLibrary > 0
                    ? "本轮已完成：\(ex.alreadyInLibrary) 部均已在库，无需新刮削。"
                    : "本轮已完成：当前暂无新增视频需要刮削。"
                await reload()
            }
        } catch {
            errorMessage = "一键增量入库失败：\(error.localizedDescription)"
            autoFlowNote = nil
        }
    }

    private func refreshConsoleState() async {
        guard let c = model.apiClient else { return }
        batchStats = try? await c.getBatchScrapeStats()
        batchProgress = try? await c.getBatchScrapeProgress()
        enrichStats = try? await c.getPerformerEnrichStats()
        enrichProgress = try? await c.getPerformerEnrichProgress()
        pipelineDiff = try? await c.getMovieSyncDiff()
        pipelineSyncProgress = try? await c.getMovieDbSyncProgress()
        if pipelineSyncProgress?.running == true {
            pipelineSyncMode = pipelineSyncProgress?.mode
            beginSyncPolling()
        }
        if batchProgress?.running == true || enrichProgress?.running == true {
            beginScrapePolling()
        }
    }

    private func beginSyncPolling() {
        syncPollTask?.cancel()
        syncPollTask = Task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(1200))
                guard let c = model.apiClient else { break }
                if let p = try? await c.getMovieDbSyncProgress() {
                    pipelineSyncProgress = p
                    if !p.running {
                        pipelineSyncMode = nil
                        if let d = p.diff { pipelineDiff = d }
                        if let err = p.error, !err.isEmpty { errorMessage = "同步失败：\(err)" }
                        await refreshConsoleState()
                        await reload()
                        break
                    }
                }
            }
        }
    }

    private func runPipelineDbSync(mode: String) async {
        guard let c = model.apiClient else { return }
        pipelineSyncMode = mode
        do {
            let p = try await c.runMovieDbSync(mode: mode)
            pipelineSyncProgress = p
            if p.running {
                beginSyncPolling()
            } else {
                pipelineSyncMode = nil
                if let d = p.diff { pipelineDiff = d }
                await refreshConsoleState()
                await reload()
            }
        } catch {
            pipelineSyncMode = nil
            errorMessage = "数据库更新失败：\(error.localizedDescription)"
        }
    }

    private func runPipelineExtract(scope: String) async {
        guard let c = model.apiClient else { return }
        pipelineExtracting = true
        defer { pipelineExtracting = false }
        do {
            pipelineExtract = try await c.extractMovieNumbers(scope: scope)
            await refreshConsoleState()
            await reload()
        } catch {
            errorMessage = "提取番号失败：\(error.localizedDescription)"
        }
    }

    private func startManualScrape(mode: String) async {
        guard let c = model.apiClient else { return }
        if mode == "failed" {
            try? await c.startFailedBatchScrape()
        } else if mode == "new" {
            try? await c.startNewBatchScrape()
        } else {
            try? await c.startBatchScrape()
        }
        batchProgress = try? await c.getBatchScrapeProgress()
        beginScrapePolling()
    }

    private func stopScrape() async {
        guard let c = model.apiClient else { return }
        batchProgress = try? await c.stopBatchScrape()
        await refreshConsoleState()
        await reload()
    }

    private func startEnrich(forceAll: Bool) async {
        guard let c = model.apiClient else { return }
        enrichProgress = try? await c.startPerformerEnrich(forceAll: forceAll)
        beginScrapePolling()
    }

    private func pauseEnrich() async {
        guard let c = model.apiClient else { return }
        enrichProgress = try? await c.pausePerformerEnrich()
    }

    private func resumeEnrich() async {
        guard let c = model.apiClient else { return }
        enrichProgress = try? await c.resumePerformerEnrich()
        beginScrapePolling()
    }

    private func stopEnrich() async {
        guard let c = model.apiClient else { return }
        enrichProgress = try? await c.stopPerformerEnrich()
        await refreshConsoleState()
        await loadDiscoveryData()
    }

    private func beginScrapePolling() {
        scrapePollTask?.cancel()
        scrapePollTask = Task { @MainActor in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                guard let c = model.apiClient else { return }
                let prevCompleted = batchProgress?.completed ?? -1
                batchProgress = try? await c.getBatchScrapeProgress()
                enrichProgress = try? await c.getPerformerEnrichProgress()
                if (batchProgress?.completed ?? -1) != prevCompleted {
                    batchStats = try? await c.getBatchScrapeStats()
                }
                if batchProgress?.running != true && enrichProgress?.running != true {
                    batchStats = try? await c.getBatchScrapeStats()
                    enrichStats = try? await c.getPerformerEnrichStats()
                    pipelineDiff = try? await c.getMovieSyncDiff()
                    await loadDiscoveryData()
                    return
                }
            }
        }
    }
}
