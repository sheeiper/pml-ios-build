import SwiftUI

/// Server-backed movie library. Search, facets, sorting and pagination are never performed locally.
public struct MovieLibraryView: View {
    @EnvironmentObject private var model: AppModel
    @State private var pageItems: [MovieMetadata] = []
    @State private var total = 0
    @State private var makers: [String] = []
    @State private var actors: [String] = []
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
    @State private var sort = "updated_at"
    @State private var mode: LibraryMode = .movies
    @State private var offset = 0
    @State private var loading = false
    @State private var loadingMore = false
    @State private var errorMessage: String?
    @State private var selectedMovie: MovieMetadata?
    @State private var selectedPerformer: MoviePerformer?
    @State private var preview: PreviewItem?
    @State private var showScrapeSheet = false
    @State private var batchStats: BatchScrapeStats?
    @State private var batchProgress: BatchScrapeProgress?
    @State private var scrapePollTask: Task<Void, Never>?
    @State private var requestVersion = 0

    private let pageSize = 40
    private enum LibraryMode: String, CaseIterable, Identifiable {
        case movies = "影视库", issues = "失败分类", unrecognized = "未识别", performers = "人物"
        var id: String { rawValue }
    }

    public init() {}

    public var body: some View {
        VStack(spacing: 0) {
            Picker("内容", selection: $mode) {
                ForEach(LibraryMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal)
            .padding(.top, 8)
            .onChange(of: mode) { _, _ in resetAndLoad() }

            if mode == .movies { movieFilters }
            else if mode == .issues { issueFilters }
            else if mode == .performers { performerSearch }

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
        .searchable(text: $searchQuery, prompt: mode == .performers ? "搜索姓名或别名" : "搜索番号、片名、女优")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button { showScrapeSheet = true } label: {
                    Label("手动刮削", systemImage: "bolt.badge.clock.fill").foregroundStyle(.purple)
                }
            }
        }
        .task { await reload() }
        .task(id: searchQuery) {
            // Debounce both typing and clearing the search field so an empty query reloads the full server page.
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
            await reload()
        }
        .refreshable { await reload() }
        .onDisappear { scrapePollTask?.cancel() }
        .sheet(item: $selectedMovie) { movie in
            MovieDetailSheet(number: movie.number, assetId: movie.assetId, fileName: movie.title, remotePath: nil) { target in
                Task { await playAsset(target, title: movie.title) }
            }
            .environmentObject(model)
        }
        .sheet(item: $selectedPerformer) { performer in
            MoviePerformerDetailSheet(performer: performer).environmentObject(model)
        }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(url: item.url, title: item.title, assetId: item.assetId, startAt: item.startAt, fallbackURL: item.fallbackURL, sourceLabel: item.sourceLabel, remotePath: item.remotePath)
                .id(item.id).environmentObject(model)
        }
        .sheet(isPresented: $showScrapeSheet) { manualScrapeSheet }
    }

    @ViewBuilder private var content: some View {
        if loading && currentCount == 0 {
            ProgressView("正在从服务器查询…").frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if currentCount == 0 {
            ContentUnavailableView(emptyTitle, systemImage: emptySymbol, description: Text(emptyDescription))
        } else {
            ScrollView {
                LazyVStack(spacing: 12) {
                    switch mode {
                    case .movies:
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
                        ForEach(performers) { person in performerCell(person).onAppear { loadMoreIfNeeded(person) } }
                    }
                    if loadingMore { ProgressView().padding() }
                }
                .padding(14)
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
        case .movies: return "没有匹配的影片"
        case .issues: return "没有失败记录"
        case .unrecognized: return "没有未识别视频"
        case .performers: return "没有人物候选"
        }
    }
    private var emptySymbol: String { mode == .performers ? "person.2" : mode == .issues ? "exclamationmark.triangle" : mode == .unrecognized ? "questionmark.folder" : "film.stack" }
    private var emptyDescription: String { mode == .movies ? "调整搜索、片厂或女优筛选。" : "数据由服务端分页返回。" }

    private var movieFilters: some View {
        VStack(spacing: 8) {
            HStack {
                Picker("排序", selection: $sort) {
                    Text("最近更新").tag("updated_at"); Text("发行日期").tag("date"); Text("番号").tag("number"); Text("标题").tag("title")
                }
                .pickerStyle(.menu)
                Spacer()
                Menu("片厂") {
                    Button("全部") { selectedMaker = ""; resetAndLoad() }
                    ForEach(makers, id: \.self) { value in Button(value) { selectedMaker = value; resetAndLoad() } }
                }
                Menu("女优") {
                    Button("全部") { selectedActor = ""; resetAndLoad() }
                    ForEach(actors, id: \.self) { value in Button(value) { selectedActor = value; resetAndLoad() } }
                }
            }
            .font(.caption)
            if !selectedMaker.isEmpty || !selectedActor.isEmpty {
                HStack {
                    if !selectedMaker.isEmpty { Text("片厂: \(selectedMaker)") }
                    if !selectedActor.isEmpty { Text("女优: \(selectedActor)") }
                    Button("清除") { selectedMaker = ""; selectedActor = ""; resetAndLoad() }
                }.font(.caption2).foregroundStyle(.secondary)
            }
        }.padding(.horizontal).onChange(of: sort) { _, _ in resetAndLoad() }
    }

    private var issueFilters: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack {
                issueChip("全部", value: "")
                ForEach(issueCounts) { item in issueChip("\(item.category) (\(item.count))", value: item.category) }
            }.padding(.horizontal)
        }.padding(.vertical, 6)
    }
    private func issueChip(_ title: String, value: String) -> some View {
        Button(title) { issueCategory = value; resetAndLoad() }
            .font(.caption).padding(.horizontal, 10).padding(.vertical, 6)
            .background(issueCategory == value ? Color.purple.opacity(0.2) : Color.secondary.opacity(0.12), in: Capsule())
    }
    private var performerSearch: some View { Text("候选人物不会自动合并；点击候选查看作品并明确确认身份").font(.caption).foregroundStyle(.secondary).padding(.horizontal) }

    private func movieCell(_ movie: MovieMetadata) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            CachedImageView(url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fill, minHeight: 100, maxPixel: 900)
                .aspectRatio(16 / 9, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            Text(movie.number).font(.caption2.bold()).foregroundStyle(.secondary)
            Text(movie.title).font(.caption.weight(.semibold)).lineLimit(2)
        }
    }
    private func issueCell(_ issue: ScrapeIssue) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack { Text(issue.number).font(.subheadline.bold()); Spacer(); Text(issue.category).font(.caption).foregroundStyle(issue.retryable ? .orange : .red) }
            Text(issue.error).font(.caption).foregroundStyle(.secondary).lineLimit(3)
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading).background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
    }
    private func unrecognizedCell(_ item: UnrecognizedVideo) -> some View {
        VStack(alignment: .leading, spacing: 4) { Text(item.fileName).font(.subheadline.weight(.semibold)); Text(item.remotePath).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
            .padding(12).frame(maxWidth: .infinity, alignment: .leading).background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
    }
    private func performerCell(_ person: MoviePerformer) -> some View {
        Button { selectedPerformer = person } label: {
            HStack(spacing: 12) {
                CachedImageView(url: person.avatarUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fill, minHeight: 52, maxPixel: 240).frame(width: 52, height: 52).clipShape(Circle())
                VStack(alignment: .leading, spacing: 3) { Text(person.name).font(.subheadline.bold()); Text(person.aliases.joined(separator: " / ")).font(.caption).foregroundStyle(.secondary).lineLimit(1); Text("\(person.movieCount) 部 · \(person.status == "confirmed" ? "已确认" : "未确认")").font(.caption2).foregroundStyle(person.status == "confirmed" ? .green : .orange) }
                Spacer(); Image(systemName: "chevron.right").foregroundStyle(.tertiary)
            }.padding(10).background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
        }.buttonStyle(.plain)
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
                let result = try await client.listMoviePerformers(query: searchQuery, limit: pageSize, offset: offset)
                guard version == requestVersion else { return }
                performers = reset ? result.items : performers + result.items; performerTotal = result.total; offset += result.items.count
            }
        } catch { if version == requestVersion { errorMessage = error.localizedDescription } }
    }

    private func playAsset(_ assetId: String, title: String) async {
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else { return }
        preview = PreviewItem(url: source.url, isVideo: true, assetId: assetId, title: title, fallbackURL: source.fallbackURL, sourceLabel: source.label, remotePath: nil)
    }

    private var manualScrapeSheet: some View {
        NavigationStack {
            Form {
                Section("手动刮削控制台") {
                    Text("仅在你明确点击开始后执行；不会自动批量确认人物或合并候选。\n白名单视频：\(batchStats?.totalWhitelistedVideos ?? 0) · 待刮削：\(batchStats?.remainingToScrape ?? 0)").font(.caption)
                    if let progress = batchProgress, progress.running { ProgressView(value: Double(progress.completed), total: Double(max(progress.total, 1))) }
                    Button(batchProgress?.running == true ? "停止" : "开始手动增量刮削") { Task { await toggleScrape() } }
                }
            }.navigationTitle("增量刮削").toolbar { ToolbarItem(placement: .topBarTrailing) { Button("关闭") { showScrapeSheet = false } } }
        }
        .task { batchStats = try? await model.apiClient?.getBatchScrapeStats(); batchProgress = try? await model.apiClient?.getBatchScrapeProgress() }
    }
    private func toggleScrape() async { if batchProgress?.running == true { await stopScrape() } else { await startManualScrape() } }
    private func startManualScrape() async { guard let c = model.apiClient else { return }; try? await c.startBatchScrape(); batchProgress = try? await c.getBatchScrapeProgress(); beginScrapePolling() }
    private func stopScrape() async { guard let c = model.apiClient else { return }; batchProgress = try? await c.stopBatchScrape(); await reload() }
    private func beginScrapePolling() { scrapePollTask?.cancel(); scrapePollTask = Task { @MainActor in while !Task.isCancelled { try? await Task.sleep(for: .seconds(2)); guard let c = model.apiClient else { return }; batchProgress = try? await c.getBatchScrapeProgress(); if batchProgress?.running == false { return } } } }
}
