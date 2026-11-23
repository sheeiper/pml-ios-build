import SwiftUI

/// Server-backed movie library. Search, facets, sorting and pagination are never performed locally.
public struct MovieLibraryView: View {
    @EnvironmentObject private var model: AppModel
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
    @State private var showScrapeSheet = false
    @State private var showFilterDrawer = false
    @State private var batchStats: BatchScrapeStats?
    @State private var batchProgress: BatchScrapeProgress?
    @State private var enrichStats: PerformerEnrichStats?
    @State private var enrichProgress: PerformerEnrichProgress?
    @State private var syncingRemote = false
    @State private var syncReport: RemoteSyncReport?
    @State private var manualTargetVideo: UnrecognizedVideo?
    @State private var manualNumberInput = ""
    @State private var scrapePollTask: Task<Void, Never>?
    @State private var requestVersion = 0

    private let pageSize = 40
    private enum LibraryMode: String, CaseIterable, Identifiable {
        case movies = "影视库", performers = "演员", issues = "失败", unrecognized = "未识别"
        var id: String { rawValue }
    }

    public init() {}

    private var hasActiveFilters: Bool {
        !searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !selectedMaker.isEmpty || !selectedActor.isEmpty
    }

    private var heroMovie: MovieMetadata? {
        guard !randomHeroPool.isEmpty else { return nil }
        return randomHeroPool[heroIndex % randomHeroPool.count]
    }

    public var body: some View {
        Group {
            if let performer = selectedPerformer {
                MoviePerformerDetailSheet(
                    performer: performer,
                    onBack: { selectedPerformer = nil },
                    onFilterMovies: { actorName in
                        selectedPerformer = nil
                        mode = .movies
                        selectedActor = actorName
                        resetAndLoad()
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

                    if mode == .movies, activeMovieFilterCount > 0 { activeMovieFilterStrip }
                    else if mode == .performers, activePerformerFilterCount > 0 { activePerformerFilterStrip }
                    else if mode == .issues { issueFilters }

                    if let errorMessage {
                        Label(errorMessage, systemImage: "exclamationmark.triangle")
                            .font(.footnote).foregroundStyle(.red)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal).padding(.vertical, 8)
                    }

                    content
                }
                .navigationTitle(mode == .movies ? "影视库 (\(total))" : mode.rawValue)
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
                        Button { showScrapeSheet = true } label: {
                            Label("数据管理", systemImage: "bolt.badge.clock.fill").foregroundStyle(.purple)
                        }
                    }
                }
                .task {
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
                }
                .onDisappear { scrapePollTask?.cancel() }
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
        .sheet(isPresented: $showScrapeSheet) { manualScrapeSheet }
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
            ProgressView("正在从服务器查询…").frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if currentCount == 0 {
            ContentUnavailableView(emptyTitle, systemImage: emptySymbol, description: Text(emptyDescription))
        } else {
            ScrollView {
                LazyVStack(spacing: 14) {
                    switch mode {
                    case .movies:
                        if !hasActiveFilters, let hero = heroMovie {
                            heroBanner(hero)
                        }
                        if !topPerformers.isEmpty {
                            castRail
                        }
                        LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 14) {
                            ForEach(pageItems) { movie in
                                Button { selectedMovie = movie } label: { movieCell(movie) }
                                    .buttonStyle(.plain)
                                    .onAppear { loadMoreIfNeeded(movie) }
                            }
                        }
                    case .issues:
                        ForEach(issues) { issue in issueCell(issue).onAppear { loadMoreIfNeeded(issue) } }
                    case .unrecognized:
                        ForEach(unrecognized) { item in unrecognizedCell(item).onAppear { loadMoreIfNeeded(item) } }
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

    private var currentCount: Int {
        switch mode {
        case .movies: return pageItems.count
        case .issues: return issues.count
        case .unrecognized: return unrecognized.count
        case .performers: return performers.count
        }
    }
    private var emptyTitle: String {
        switch mode {
        case .movies: return "暂无匹配的影视作品"
        case .issues: return "暂无刮削失败记录"
        case .unrecognized: return "暂无无法识别视频"
        case .performers: return "暂无演员档案"
        }
    }
    private var emptySymbol: String {
        switch mode {
        case .movies: return "film.stack"
        case .issues: return "checkmark.shield"
        case .unrecognized: return "doc.questionmark"
        case .performers: return "person.2"
        }
    }
    private var emptyDescription: String {
        switch mode {
        case .movies: return "服务端仅返回已验证影视记录；点击右上角数据管理可增量同步与刮削。"
        case .issues: return "失败记录按原因分类，不与未识别视频混算。"
        case .unrecognized: return "文件名与父目录均无法提取番号的白名单视频会列在这里。"
        case .performers: return "完成影片刮削后会自动入库演员档案。"
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
        (sort != "release_date_desc" ? 1 : 0) + (!selectedMaker.isEmpty ? 1 : 0) + (!selectedActor.isEmpty ? 1 : 0)
    }

    private var activePerformerFilterCount: Int {
        (performerSort != "latest_release" ? 1 : 0)
            + (!performerAgeRange.isEmpty ? 1 : 0)
            + (!performerHeightRange.isEmpty ? 1 : 0)
            + (!performerCup.isEmpty ? 1 : 0)
            + (performerMinMovies > 0 ? 1 : 0)
            + (!performerProfileFilter.isEmpty ? 1 : 0)
    }

    private var activeMovieFilterStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
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
                            sort = "release_date_desc"
                            selectedMaker = ""
                            selectedActor = ""
                        } else {
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
        VStack(alignment: .leading, spacing: 5) {
            ZStack(alignment: .topLeading) {
                Color.black.opacity(0.88)
                CachedImageView(url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fit, minHeight: 108, maxPixel: 720)
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
        Button { selectedPerformer = person } label: {
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
        }
        .buttonStyle(.plain)
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
        offset = 0; pageItems = []; issues = []; unrecognized = []; performers = []; errorMessage = nil
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
                let result = try await client.queryMovieLibrary(query: searchQuery, maker: selectedMaker, actor: selectedActor, sort: sort, limit: pageSize, offset: offset)
                guard version == requestVersion else { return }
                pageItems = reset ? result.items : pageItems + result.items; total = result.total; makers = result.makers; actors = result.actors; offset += result.items.count
            case .issues:
                let result = try await client.listScrapeIssues(category: issueCategory, query: searchQuery, limit: pageSize, offset: offset)
                guard version == requestVersion else { return }
                issues = reset ? result.items : issues + result.items; issueTotal = result.total; issueCounts = result.counts; offset += result.items.count
            case .unrecognized:
                let result = try await client.listUnrecognizedVideos(limit: pageSize, offset: offset)
                guard version == requestVersion else { return }
                unrecognized = reset ? result.items : unrecognized + result.items; unrecognizedTotal = result.total; offset += result.items.count
            case .performers:
                let result = try await client.listMoviePerformers(
                    query: searchQuery,
                    sort: performerSort,
                    minMovies: performerMinMovies > 0 ? performerMinMovies : nil,
                    ageRange: performerAgeRange,
                    heightRange: performerHeightRange,
                    cup: performerCup,
                    profileFilter: performerProfileFilter,
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
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else { return }
        preview = PreviewItem(url: source.url, isVideo: true, assetId: assetId, title: title, fallbackURL: source.fallbackURL, sourceLabel: source.label, remotePath: nil)
    }

    private var manualScrapeSheet: some View {
        NavigationStack {
            Form {
                Section("影片元数据刮削 & 115 同步") {
                    Text("白名单视频：\(batchStats?.totalWhitelistedVideos ?? 0) · 已入库：\(batchStats?.alreadyScraped ?? 0) · 待刮削：\(batchStats?.remainingToScrape ?? 0) · 失败：\(batchStats?.failedCount ?? 0)")
                        .font(.caption)
                    if let progress = batchProgress, progress.running {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("正在刮削：\(progress.currentNumber ?? "准备中…") (\(progress.completed)/\(progress.total))")
                                .font(.caption2)
                            ProgressView(value: Double(progress.completed), total: Double(max(progress.total, 1)))
                        }
                    }
                    if let report = syncReport {
                        Text("同步完成：新增 \(report.addedFiles) · 移除/改名 \(report.removedFiles) · 清理旧失败 \(report.cleanedStaleFailures) · 待刮削新番号 \(report.pendingNewCount)")
                            .font(.caption2)
                            .foregroundStyle(.green)
                    }
                    if batchProgress?.running == true {
                        Button("停止影片刮削", role: .destructive) { Task { await stopScrape() } }
                    } else {
                        Button {
                            Task { await runRemoteSync(autoScrape: true, mode: "fast") }
                        } label: {
                            Label(syncingRemote ? "正在同步 115…" : "一键同步 115 变动并刮新增（推荐）", systemImage: "bolt.fill")
                                .font(.body.weight(.semibold))
                        }
                        .disabled(syncingRemote)
                        .tint(.purple)

                        Button("仅刮变动/新增（跳过失败项）") { Task { await startManualScrape(mode: "new") } }

                        DisclosureGroup("更多高级运维选项...") {
                            Button("快速检测 115 变动") {
                                Task { await runRemoteSync(autoScrape: false, mode: "fast") }
                            }
                            .disabled(syncingRemote)
                            Button("仅刷新失败项目录") {
                                Task { await runRemoteSync(autoScrape: false, mode: "issues") }
                            }
                            .disabled(syncingRemote)
                            Button("深度全量校验") {
                                Task { await runRemoteSync(autoScrape: false, mode: "deep") }
                            }
                            .disabled(syncingRemote)
                            Button("全量增量刮削（含失败项）") { Task { await startManualScrape(mode: "all") } }
                            if (batchStats?.failedCount ?? 0) > 0 {
                                Button("仅重试失败项（含 FC2 兜底）") { Task { await startManualScrape(mode: "failed") } }
                            }
                        }
                        .font(.subheadline)
                    }
                }

                Section("演员头像与档案补全") {
                    let totalP = enrichStats?.totalPerformers ?? 0
                    let withAv = enrichStats?.withAvatar ?? 0
                    let pct = totalP > 0 ? Int(Double(withAv) / Double(totalP) * 100) : 0
                    Text("总演员：\(totalP) · 已有头像：\(withAv) (\(pct)%) · 待补全：\(enrichStats?.missingCount ?? 0)")
                        .font(.caption)
                    if let ep = enrichProgress, ep.running {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("正在获取：\(ep.currentName ?? "扫描中…") (\(ep.completed)/\(ep.total) · 成功 \(ep.enriched))")
                                .font(.caption2)
                            ProgressView(value: Double(ep.completed), total: Double(max(ep.total, 1)))
                        }
                        if ep.paused {
                            Button("继续演员补全") { Task { await resumeEnrich() } }
                        } else {
                            Button("暂停") { Task { await pauseEnrich() } }
                        }
                        Button("停止演员补全", role: .destructive) { Task { await stopEnrich() } }
                    } else {
                        if let msg = enrichProgress?.lastMessage, !msg.isEmpty {
                            Text(msg).font(.caption2).foregroundStyle(.secondary)
                        }
                        Button("补全缺失演员资料") { Task { await startEnrich(forceAll: false) } }
                        Button("强制刷新全部演员") { Task { await startEnrich(forceAll: true) } }
                    }
                }
            }
            .navigationTitle("数据管理")
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("关闭") { showScrapeSheet = false } } }
        }
        .task { await refreshConsoleState() }
    }

    private func refreshConsoleState() async {
        guard let c = model.apiClient else { return }
        batchStats = try? await c.getBatchScrapeStats()
        batchProgress = try? await c.getBatchScrapeProgress()
        enrichStats = try? await c.getPerformerEnrichStats()
        enrichProgress = try? await c.getPerformerEnrichProgress()
        if batchProgress?.running == true || enrichProgress?.running == true {
            beginScrapePolling()
        }
    }

    private func runRemoteSync(autoScrape: Bool, mode: String) async {
        guard let c = model.apiClient else { return }
        syncingRemote = true
        defer { syncingRemote = false }
        do {
            syncReport = try await c.syncRemoteAndReconcile(autoScrape: autoScrape, mode: mode)
            await refreshConsoleState()
            await reload()
        } catch {
            errorMessage = "同步 115 失败：\(error.localizedDescription)"
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
                batchProgress = try? await c.getBatchScrapeProgress()
                enrichProgress = try? await c.getPerformerEnrichProgress()
                if batchProgress?.running != true && enrichProgress?.running != true {
                    batchStats = try? await c.getBatchScrapeStats()
                    enrichStats = try? await c.getPerformerEnrichStats()
                    await loadDiscoveryData()
                    return
                }
            }
        }
    }
}
