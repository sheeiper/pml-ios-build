import SwiftUI

public struct MovieLibraryView: View {
    @EnvironmentObject private var model: AppModel

    @State private var movies: [MovieMetadata] = []
    @State private var loading = false
    @State private var searchQuery = ""
    @State private var selectedMaker: String?
    @State private var selectedActor: String?
    @State private var selectedMovie: MovieMetadata?
    @State private var showScrapeSheet = false
    @State private var batchStats: BatchScrapeStats?
    @State private var batchProgress: BatchScrapeProgress?

    private let columns = [
        GridItem(.adaptive(minimum: 155, maximum: 200), spacing: 14)
    ]

    public init() {}

    private var filteredMovies: [MovieMetadata] {
        var list = movies
        let q = searchQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !q.isEmpty {
            list = list.filter {
                $0.number.lowercased().contains(q) ||
                $0.title.lowercased().contains(q) ||
                $0.actors.contains { $0.lowercased().contains(q) } ||
                ($0.maker?.lowercased().contains(q) ?? false)
            }
        }
        if let maker = selectedMaker {
            list = list.filter { $0.maker == maker }
        }
        if let actor = selectedActor {
            list = list.filter { $0.actors.contains(actor) }
        }
        return list
    }

    private var availableMakers: [String] {
        let set = Set(movies.compactMap { $0.maker })
        return Array(set).sorted()
    }

    private var availableActors: [String] {
        var set = Set<String>()
        for m in movies {
            for a in m.actors {
                set.insert(a)
            }
        }
        return Array(set).sorted()
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                // 正在运行时的简易状态提示
                if let prog = batchProgress, prog.running {
                    HStack {
                        ProgressView()
                            .controlSize(.small)
                        Text(prog.paused ? "增量刮削已暂停" : "正在温和刮削: \(prog.currentNumber ?? "")")
                            .font(.caption.bold())
                            .foregroundStyle(.purple)
                        Spacer()
                        Text("\(prog.completed)/\(prog.total)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.purple.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
                    .padding(.horizontal)
                }

                // 筛选胶囊水平滚动
                if !availableMakers.isEmpty || !availableActors.isEmpty {
                    filterChips
                }

                if loading && movies.isEmpty {
                    VStack(spacing: 12) {
                        ProgressView()
                        Text("正在载入影视库海报墙...")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 80)
                } else if filteredMovies.isEmpty {
                    VStack(spacing: 12) {
                        Image(systemName: "film")
                            .font(.system(size: 44))
                            .foregroundStyle(.secondary)
                        Text(movies.isEmpty ? "影视库暂无入库影片" : "未找到匹配的影片")
                            .font(.headline)
                        Text(movies.isEmpty ? "请点击右上角【手动刮削】按钮，从白名单目录中抓取官方海报与百科。" : "请尝试更改搜索关键词或重置筛选。")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 32)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 60)
                } else {
                    // Emby 风格官方高清海报网格
                    LazyVGrid(columns: columns, spacing: 16) {
                        ForEach(filteredMovies) { m in
                            Button {
                                selectedMovie = m
                            } label: {
                                posterCell(m)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal)
                }
            }
            .padding(.vertical, 8)
        }
        .navigationTitle("影视库 (\(filteredMovies.count))")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $searchQuery, prompt: "搜索番号、片名或女优")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showScrapeSheet = true
                } label: {
                    Label("手动刮削", systemImage: "bolt.badge.clock.fill")
                }
            }
        }
        .task {
            await loadData()
        }
        .sheet(item: $selectedMovie) { m in
            MovieDetailSheet(
                number: m.number,
                assetId: m.assetId,
                fileName: m.title,
                remotePath: nil
            ) { _ in
                // 在详情中点击播放时
            }
            .environmentObject(model)
        }
        .sheet(isPresented: $showScrapeSheet) {
            manualScrapeSheet
        }
    }

    private func posterCell(_ m: MovieMetadata) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack(alignment: .topLeading) {
                // 封面
                if let cover = m.coverUrl, let url = model.proxyScraperImageURL(rawUrl: cover) {
                    AsyncImage(url: url) { phase in
                        switch phase {
                        case .success(let image):
                            image
                                .resizable()
                                .scaledToFill()
                        case .failure:
                            Rectangle().fill(Color.secondary.opacity(0.15))
                        case .empty:
                            Rectangle().fill(Color.secondary.opacity(0.1))
                        @unknown default:
                            EmptyView()
                        }
                    }
                } else {
                    Rectangle().fill(Color.secondary.opacity(0.15))
                }

                // 左上角：番号药丸
                Text(m.number)
                    .font(.system(size: 9.5, weight: .bold))
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2.5)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 4))
                    .foregroundStyle(.white)
                    .padding(5)

                // 右上角：年份
                if let date = m.releaseDate, let yr = date.split(separator: "-").first {
                    Text(String(yr))
                        .font(.system(size: 9, weight: .semibold))
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(Color.black.opacity(0.55), in: RoundedRectangle(cornerRadius: 3))
                        .foregroundStyle(.white)
                        .padding(5)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
                }
            }
            .aspectRatio(2 / 3, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
            .shadow(color: .black.opacity(0.2), radius: 5, y: 3)

            // 文本信息
            Text(m.title)
                .font(.system(size: 11.5, weight: .semibold))
                .lineLimit(2)
                .foregroundStyle(.primary)

            if !m.actors.isEmpty {
                Text(m.actors.joined(separator: " / "))
                    .font(.system(size: 10))
                    .foregroundStyle(.purple)
                    .lineLimit(1)
            } else if let maker = m.maker {
                Text(maker)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private var filterChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                if selectedMaker != nil || selectedActor != nil {
                    Button {
                        selectedMaker = nil
                        selectedActor = nil
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "xmark")
                            Text("重置筛选")
                        }
                        .font(.caption2.bold())
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(Color.red.opacity(0.12), in: Capsule())
                        .foregroundStyle(.red)
                    }
                }

                // 片厂筛选
                ForEach(availableMakers.prefix(15), id: \.self) { maker in
                    Button {
                        selectedMaker = (selectedMaker == maker) ? nil : maker
                    } label: {
                        Text(maker)
                            .font(.caption2.bold())
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(selectedMaker == maker ? Color.purple : Color.secondary.opacity(0.12), in: Capsule())
                            .foregroundStyle(selectedMaker == maker ? .white : .primary)
                    }
                }

                // 女优筛选
                ForEach(availableActors.prefix(20), id: \.self) { actor in
                    Button {
                        selectedActor = (selectedActor == actor) ? nil : actor
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "heart.fill")
                                .font(.system(size: 8))
                                .foregroundStyle(selectedActor == actor ? .white : .pink)
                            Text(actor)
                        }
                        .font(.caption2.bold())
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(selectedActor == actor ? Color.pink : Color.secondary.opacity(0.12), in: Capsule())
                        .foregroundStyle(selectedActor == actor ? .white : .primary)
                    }
                }
            }
            .padding(.horizontal)
        }
    }

    private var manualScrapeSheet: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("系统将扫描您在【治理中心】设定的白名单目录，排查未刮削的日本影片，以约 2 秒间隔温和抓取官方海报、女优与剧照。")
                            .font(.caption)
                            .foregroundStyle(.secondary)

                        HStack(spacing: 12) {
                            VStack(alignment: .leading) {
                                Text("\(batchStats?.totalWhitelistedVideos ?? 0)").font(.headline.bold())
                                Text("白名单视频").font(.caption2).foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .leading) {
                                Text("\(batchStats?.distinctNumbers ?? 0)").font(.headline.bold())
                                Text("唯一番号").font(.caption2).foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .leading) {
                                Text("\(batchStats?.alreadyScraped ?? 0)").font(.headline.bold()).foregroundStyle(.green)
                                Text("已刮削").font(.caption2).foregroundStyle(.secondary)
                            }
                            Spacer()
                            VStack(alignment: .leading) {
                                Text("\(batchStats?.remainingToScrape ?? 0)").font(.headline.bold()).foregroundStyle(.purple)
                                Text("待刮削").font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 4)

                        if let prog = batchProgress, prog.running {
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text(prog.paused ? "已暂停" : "正在刮削中...")
                                        .font(.subheadline.bold())
                                        .foregroundStyle(prog.paused ? .orange : .purple)
                                    Spacer()
                                    Text("\(prog.completed)/\(prog.total)")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                                if let cur = prog.currentNumber {
                                    Text("正在刮削: \(cur)")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                                ProgressView(value: Double(prog.completed), total: Double(max(prog.total, 1)))
                                    .tint(prog.paused ? .orange : .purple)
                            }
                        }

                        HStack(spacing: 12) {
                            if batchProgress?.running != true {
                                Button {
                                    Task { await startManualScrape() }
                                } label: {
                                    Label("开始手动增量刮削", systemImage: "bolt.fill")
                                }
                                .buttonStyle(.borderedProminent)
                                .tint(.purple)
                            } else {
                                if batchProgress?.paused == true {
                                    Button("继续") {
                                        Task { await resumeScrape() }
                                    }
                                    .buttonStyle(.bordered)
                                    .tint(.green)
                                } else {
                                    Button("暂停") {
                                        Task { await pauseScrape() }
                                    }
                                    .buttonStyle(.bordered)
                                    .tint(.orange)
                                }

                                Button("终止", role: .destructive) {
                                    Task { await stopScrape() }
                                }
                                .buttonStyle(.bordered)
                            }
                        }
                        .padding(.top, 6)
                    }
                } header: {
                    Label("手动刮削控制台", systemImage: "bolt.badge.clock.fill")
                }
            }
            .navigationTitle("增量刮削管理")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("关闭") {
                        showScrapeSheet = false
                    }
                }
            }
        }
    }

    private func loadData() async {
        guard let client = model.apiClient else { return }
        loading = true
        do {
            let list = try await client.listMovieMetadata(limit: 1000, offset: 0)
            self.movies = list.filter { $0.source != "javdb_not_found" }
            self.batchStats = try? await client.getBatchScrapeStats()
            self.batchProgress = try? await client.getBatchScrapeProgress()
        } catch {
            print("Failed to load movies:", error)
        }
        loading = false
    }

    private func startManualScrape() async {
        guard let client = model.apiClient else { return }
        do {
            try await client.startBatchScrape()
            batchProgress = try? await client.getBatchScrapeProgress()
        } catch {
            print("Failed to start batch scrape:", error)
        }
    }

    private func pauseScrape() async {
        guard let client = model.apiClient else { return }
        batchProgress = try? await client.pauseBatchScrape()
    }

    private func resumeScrape() async {
        guard let client = model.apiClient else { return }
        batchProgress = try? await client.resumeBatchScrape()
    }

    private func stopScrape() async {
        guard let client = model.apiClient else { return }
        batchProgress = try? await client.stopBatchScrape()
        await loadData()
    }
}
