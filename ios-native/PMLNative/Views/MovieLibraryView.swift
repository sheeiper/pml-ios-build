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
    @State private var preview: PreviewItem?

    // 手机端黄金 2 列网格；iPad 端根据宽度自动扩展
    private let columns = [
        GridItem(.flexible(), spacing: 12),
        GridItem(.flexible(), spacing: 12)
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
                // 正在运行时的悬浮状态提示
                if let prog = batchProgress, prog.running {
                    HStack(spacing: 8) {
                        ProgressView()
                            .controlSize(.small)
                            .tint(.purple)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(prog.paused ? "增量刮削已暂停" : "正在温和防封刮削")
                                .font(.caption.bold())
                                .foregroundStyle(.primary)
                            if let cur = prog.currentNumber {
                                Text("番号: \(cur)")
                                    .font(.caption2)
                                    .foregroundStyle(.purple)
                            }
                        }
                        Spacer()
                        Text("\(prog.completed) / \(prog.total)")
                            .font(.caption2.bold())
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.purple.opacity(0.12), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .stroke(Color.purple.opacity(0.25), lineWidth: 1)
                    )
                    .padding(.horizontal)
                }

                // 热门女优快捷横向筛选轨（女优专区）
                if !availableActors.isEmpty {
                    hotActorsTrack
                }

                // 片厂与综合筛选
                if !availableMakers.isEmpty || selectedActor != nil || selectedMaker != nil {
                    filterChips
                }

                if loading && movies.isEmpty {
                    VStack(spacing: 12) {
                        ProgressView()
                            .controlSize(.regular)
                            .tint(.purple)
                        Text("正在载入影视百科海报墙...")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 80)
                } else if filteredMovies.isEmpty {
                    emptyStateView
                } else {
                    // 流媒体风格 2 列官方海报流
                    LazyVGrid(columns: columns, spacing: 14) {
                        ForEach(filteredMovies) { m in
                            Button {
                                selectedMovie = m
                            } label: {
                                posterCell(m)
                            }
                            .buttonStyle(PosterCardButtonStyle())
                        }
                    }
                    .padding(.horizontal, 14)
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
                        .foregroundStyle(Color.purple)
                }
            }
        }
        .task {
            await loadData()
        }
        .refreshable {
            await loadData()
        }
        .sheet(item: $selectedMovie) { m in
            MovieDetailSheet(
                number: m.number,
                assetId: m.assetId,
                fileName: m.title,
                remotePath: nil
            ) { targetId in
                Task { await playAsset(targetId, title: m.title) }
            }
            .environmentObject(model)
        }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(
                url: item.url,
                title: item.title,
                assetId: item.assetId,
                startAt: item.startAt,
                fallbackURL: item.fallbackURL,
                sourceLabel: item.sourceLabel,
                remotePath: item.remotePath
            )
            .id(item.id)
            .environmentObject(model)
        }
        .sheet(isPresented: $showScrapeSheet) {
            manualScrapeSheet
        }
    }

    // MARK: - 热门女优横向快捷条
    private var hotActorsTrack: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 4) {
                Image(systemName: "heart.fill")
                    .font(.caption2)
                    .foregroundStyle(.pink)
                Text("女优专区")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(availableActors.prefix(30), id: \.self) { actor in
                        let isSelected = selectedActor == actor
                        Button {
                            selectedActor = isSelected ? nil : actor
                        } label: {
                            Text(actor)
                                .font(.caption2.weight(isSelected ? .bold : .medium))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 5)
                                .background(
                                    isSelected ?
                                    AnyShapeStyle(LinearGradient(colors: [.pink, .roseRed], startPoint: .topLeading, endPoint: .bottomTrailing)) :
                                    AnyShapeStyle(Color.secondary.opacity(0.12)),
                                    in: Capsule()
                                )
                                .foregroundStyle(isSelected ? .white : .primary)
                        }
                    }
                }
                .padding(.horizontal)
            }
        }
    }

    // MARK: - 片厂与辅助筛选
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
                        .padding(.horizontal, 9)
                        .padding(.vertical, 5)
                        .background(Color.red.opacity(0.12), in: Capsule())
                        .foregroundStyle(.red)
                    }
                }

                // 片厂筛选
                ForEach(availableMakers.prefix(15), id: \.self) { maker in
                    let isSelected = selectedMaker == maker
                    Button {
                        selectedMaker = isSelected ? nil : maker
                    } label: {
                        Text(maker)
                            .font(.caption2.weight(isSelected ? .bold : .medium))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(
                                isSelected ?
                                AnyShapeStyle(LinearGradient(colors: [.purple, .indigo], startPoint: .topLeading, endPoint: .bottomTrailing)) :
                                AnyShapeStyle(Color.secondary.opacity(0.12)),
                                in: Capsule()
                            )
                            .foregroundStyle(isSelected ? .white : .primary)
                    }
                }
            }
            .padding(.horizontal)
        }
    }

    // MARK: - 空状态视图
    private var emptyStateView: some View {
        VStack(spacing: 14) {
            Image(systemName: "film.stack")
                .font(.system(size: 48))
                .foregroundStyle(.purple.opacity(0.8))
            Text(movies.isEmpty ? "影视库暂未入库影片" : "未找到匹配的影片")
                .font(.headline)
            Text(movies.isEmpty ? "请点击下方或右上角【手动刮削】，系统将基于白名单目录智能抓取官方高清海报与百科。" : "请尝试更改搜索关键词或重置筛选。")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)

            if movies.isEmpty {
                Button {
                    showScrapeSheet = true
                } label: {
                    Label("立即手动增量刮削", systemImage: "bolt.fill")
                        .font(.subheadline.bold())
                        .padding(.horizontal, 18)
                        .padding(.vertical, 10)
                        .background(LinearGradient(colors: [.purple, .indigo], startPoint: .leading, endPoint: .trailing), in: Capsule())
                        .foregroundStyle(.white)
                        .shadow(color: .purple.opacity(0.35), radius: 8, y: 3)
                }
                .padding(.top, 4)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 60)
    }

    // MARK: - 官方海报卡片 (2 列黄金比例)
    private func posterCell(_ m: MovieMetadata) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack(alignment: .topLeading) {
                // 封面海报
                if let cover = m.coverUrl, let url = model.proxyScraperImageURL(rawUrl: cover) {
                    AsyncImage(url: url) { phase in
                        switch phase {
                        case .success(let image):
                            image
                                .resizable()
                                .scaledToFill()
                        case .failure:
                            Rectangle().fill(Color.secondary.opacity(0.18))
                                .overlay(Image(systemName: "film").foregroundStyle(.secondary))
                        case .empty:
                            Rectangle().fill(Color.secondary.opacity(0.12))
                                .overlay(ProgressView().controlSize(.small))
                        @unknown default:
                            EmptyView()
                        }
                    }
                } else {
                    Rectangle().fill(Color.secondary.opacity(0.18))
                        .overlay(Image(systemName: "film").foregroundStyle(.secondary))
                }

                // 底部半透明暗黑渐变遮罩
                LinearGradient(
                    colors: [.clear, Color.black.opacity(0.75)],
                    startPoint: .center,
                    endPoint: .bottom
                )

                // 左上角：番号药丸
                Text(m.number)
                    .font(.system(size: 9.5, weight: .bold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                    .foregroundStyle(.white)
                    .shadow(color: .black.opacity(0.4), radius: 3, y: 1)
                    .padding(6)

                // 右上角：来源/年份彩色标签
                sourceOrYearBadge(m)
                    .padding(6)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)

                // 悬浮/海报底部女优小标
                if !m.actors.isEmpty {
                    HStack(spacing: 3) {
                        Image(systemName: "heart.fill")
                            .font(.system(size: 8))
                            .foregroundStyle(.pink)
                        Text(m.actors.joined(separator: " / "))
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                    }
                    .padding(6)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                }
            }
            .aspectRatio(2 / 3, contentMode: .fill)
            .clipped()
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .shadow(color: .black.opacity(0.22), radius: 6, y: 3)

            // 卡片下部文本信息
            Text(m.title)
                .font(.system(size: 12, weight: .semibold))
                .lineLimit(2)
                .foregroundStyle(.primary)

            if m.actors.isEmpty, let maker = m.maker {
                Text(maker)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    @ViewBuilder
    private func sourceOrYearBadge(_ m: MovieMetadata) -> some View {
        if m.source == "caribbeancom" {
            Text("加勒比")
                .font(.system(size: 8.5, weight: .bold))
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(LinearGradient(colors: [.blue, .cyan], startPoint: .leading, endPoint: .trailing), in: RoundedRectangle(cornerRadius: 4))
                .foregroundStyle(.white)
        } else if m.source == "1pondo" {
            Text("一本道")
                .font(.system(size: 8.5, weight: .bold))
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(LinearGradient(colors: [.orange, .yellow], startPoint: .leading, endPoint: .trailing), in: RoundedRectangle(cornerRadius: 4))
                .foregroundStyle(.white)
        } else if m.number.hasPrefix("FC2-") {
            Text("FC2")
                .font(.system(size: 8.5, weight: .bold))
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(LinearGradient(colors: [.red, .pink], startPoint: .leading, endPoint: .trailing), in: RoundedRectangle(cornerRadius: 4))
                .foregroundStyle(.white)
        } else if let date = m.releaseDate, let yr = date.split(separator: "-").first {
            Text(String(yr))
                .font(.system(size: 8.5, weight: .semibold))
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(Color.black.opacity(0.6), in: RoundedRectangle(cornerRadius: 4))
                .foregroundStyle(.white)
        }
    }

    // MARK: - 手动刮削管理控制台
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
            // 过滤无封面或失败占位
            self.movies = list.filter {
                $0.source != "javdb_not_found" &&
                !($0.coverUrl ?? "").isEmpty
            }
            self.batchStats = try? await client.getBatchScrapeStats()
            self.batchProgress = try? await client.getBatchScrapeProgress()
        } catch {
            print("Failed to load movies:", error)
        }
        loading = false
    }

    private func playAsset(_ assetId: String, title: String) async {
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else { return }
        preview = PreviewItem(
            url: source.url,
            isVideo: true,
            assetId: assetId,
            title: title,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: nil
        )
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

// 触摸微缩微动效
private struct PosterCardButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.96 : 1.0)
            .animation(.easeOut(duration: 0.15), value: configuration.isPressed)
    }
}

private extension Color {
    static let roseRed = Color(red: 0.96, green: 0.25, blue: 0.37)
}
