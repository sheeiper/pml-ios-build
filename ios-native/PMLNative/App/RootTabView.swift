import SwiftUI

enum AppTab: Hashable {
    case home
    case mediaLibrary
    case shorts
    case creators
    case appCenter
}

struct RootTabView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var selection: AppTab = .home

    var body: some View {
        // 使用系统 TabView：使用 iOS 26 SDK 构建后，TabBar 会自动采用 Liquid Glass 悬浮样式。
        TabView(selection: $selection) {
            HomeView()
                .tabItem { Label("主页", systemImage: "house.fill") }
                .tag(AppTab.home)

            NavigationStack {
                MovieLibraryView()
            }
            .tabItem { Label("影视库", systemImage: "film.stack.fill") }
            .tag(AppTab.mediaLibrary)

            ShortVideosView()
                .tabItem { Label("短视频", systemImage: "play.rectangle.fill") }
                .tag(AppTab.shorts)

            AuthorsView()
                .tabItem { Label("创作者", systemImage: "person.2.fill") }
                .tag(AppTab.creators)

            NavigationStack {
                AppCenterView()
            }
            .tabItem { Label("应用中心", systemImage: "square.grid.2x2.fill") }
            .tag(AppTab.appCenter)
        }
        .id(model.settingsRevision)
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            Task { await model.reconnectIfNeeded() }
        }
    }
}

struct PlaceholderScreen: View {
    let title: String
    let subtitle: String

    var body: some View {
        NavigationStack {
            VStack(spacing: 14) {
                Text(title)
                    .font(.largeTitle.weight(.bold))
                Text(subtitle)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(32)
            .navigationTitle(title)
        }
    }
}

struct HomePlaceholderView: View {
    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text("媒体源连接后，这里会显示新增文件、常看创作者与漫画馆藏。")
                        .font(.callout)
                        .foregroundStyle(.secondary)

                    HStack(spacing: 14) {
                        statCard(title: "媒体", value: "—", symbol: "photo.on.rectangle")
                        statCard(title: "漫画", value: "—", symbol: "book.closed.fill")
                        statCard(title: "创作者", value: "—", symbol: "person.2.fill")
                    }
                }
                .padding(20)
            }
            .navigationTitle("首页")
        }
    }

    private func statCard(title: String, value: String, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Image(systemName: symbol)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(.tint)
            Text(value)
                .font(.title2.weight(.semibold))
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .pmlGlassCard(cornerRadius: 20)
    }
}

struct ProfilePlaceholderView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(spacing: 8) {
                        Image(systemName: "play.rectangle.on.rectangle")
                            .font(.system(size: 36, weight: .semibold))
                            .foregroundStyle(.tint)
                        Text("私人媒体库").font(.headline)
                        Text("版本 \(appVersion)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                }

                Section {
                    NavigationLink {
                        AppCenterView()
                    } label: {
                        Label("应用中心", systemImage: "square.grid.2x2.fill")
                    }
                }
            }
            .navigationTitle("我的")
        }
    }

    private var appVersion: String {
        (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "2.0.0"
    }
}

struct AppCenterView: View {
    private enum Destination: String, Identifiable {
        case movies, comics, magnets, collections, browse, pending, history, favorites, duplicates, scan, settings, telegram
        var id: String { rawValue }
    }

    private let columns = [GridItem(.adaptive(minimum: 150), spacing: 12)]
    @State private var destination: Destination?

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 12) {
                tile("漫画与图集", "book.closed.fill", .comics)
                tile("磁力搜索", "link.circle.fill", .magnets)
                tile("专题合集", "square.stack.3d.up.fill", .collections)
                tile("文件浏览", "folder.fill", .browse)
                tile("Telegram 订阅", "paperplane.fill", .telegram)
                tile("待整理", "tray.and.arrow.down.fill", .pending)
                tile("播放记录", "clock.arrow.circlepath", .history)
                tile("收藏", "heart.fill", .favorites)
                tile("重复文件", "doc.on.doc.fill", .duplicates)
                tile("扫描中心", "arrow.clockwise.circle.fill", .scan)
                tile("设置", "gearshape.fill", .settings)
            }
            .padding(16)
        }
        .navigationTitle("应用中心")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(item: $destination) { dest in
            switch dest {
            case .movies: MovieLibraryView()
            case .comics: ComicLibraryView()
            case .magnets: MagnetSearchView()
            case .collections: CollectionsView()
            case .browse: BrowseView()
            case .telegram: TelegramFeedView()
            case .pending: PendingCurationView()
            case .history: HistoryView()
            case .favorites: FavoritesView()
            case .duplicates: DuplicatesView()
            case .scan: ScanCenterView()
            case .settings: SettingsView()
            }
        }
    }

    private func tile(_ title: String, _ symbol: String, _ dest: Destination) -> some View {
        Button {
            destination = dest
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                Image(systemName: symbol)
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.tint)
                Text(title).font(.subheadline.weight(.semibold))
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: 96, alignment: .leading)
            .padding(14)
            .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            .pmlGlassCard(cornerRadius: 18)
        }
        .buttonStyle(PressableTileStyle())
    }
}

struct PressableTileStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.94 : 1)
            .opacity(configuration.isPressed ? 0.82 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

// MARK: - 磁力内部文件列表弹窗（全站共享）

struct MagnetFilesSheetTarget: Identifiable {
    var id: String { infoHash }
    let infoHash: String
    let title: String
}

struct MagnetFilesSheetView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let target: MagnetFilesSheetTarget

    @State private var loading = true
    @State private var errorMsg = ""
    @State private var data: MagnetFilesResponse?

    var body: some View {
        NavigationStack {
            List {
                if loading {
                    Section {
                        HStack(spacing: 10) {
                            ProgressView()
                            Text("正在查询公开种子元数据…").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                } else if !errorMsg.isEmpty {
                    Section {
                        Text(errorMsg).font(.caption).foregroundStyle(.orange)
                    }
                } else if let data {
                    Section("文件概览") {
                        HStack {
                            Text("共 \(data.fileCount) 个文件").font(.caption.bold())
                            Spacer()
                            Text(data.totalSizeText).font(.caption.bold()).foregroundStyle(.purple)
                            Text(data.source).font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    Section("文件列表") {
                        ForEach(data.files) { f in
                            HStack(alignment: .top, spacing: 8) {
                                Text(f.path).font(.caption).lineLimit(3)
                                Spacer()
                                Text(f.sizeText).font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .navigationTitle("磁力文件列表")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("关闭") { dismiss() }
                }
            }
            .task {
                guard let client = model.apiClient else { return }
                loading = true
                errorMsg = ""
                do {
                    data = try await client.inspectMagnetFiles(infoHash: target.infoHash)
                } catch {
                    errorMsg = error.localizedDescription
                }
                loading = false
            }
        }
    }
}

// MARK: - 全网磁力聚合搜索（独立应用）

@MainActor
final class CopiedMagnetsStore: ObservableObject {
    static let shared = CopiedMagnetsStore()
    private let storageKey = "pml.copied_magnets.v1"
    @Published private(set) var keys: Set<String> = []
    private var ordered: [String] = []

    private init() {
        if let saved = UserDefaults.standard.stringArray(forKey: storageKey) {
            ordered = Array(saved.prefix(500))
            keys = Set(ordered)
        }
    }

    private func normalize(_ raw: String) -> String {
        let s = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if let range = s.range(of: "btih:", options: .caseInsensitive) {
            let tail = s[range.upperBound...]
            let hash = tail.prefix { $0 != "&" }
            if !hash.isEmpty { return String(hash).lowercased() }
        }
        return s.lowercased()
    }

    func isCopied(_ hashOrMagnet: String) -> Bool {
        let k = normalize(hashOrMagnet)
        return !k.isEmpty && keys.contains(k)
    }

    func markCopied(_ hashOrMagnet: String) {
        let k = normalize(hashOrMagnet)
        guard !k.isEmpty else { return }
        ordered.removeAll { $0 == k }
        ordered.insert(k, at: 0)
        if ordered.count > 500 { ordered = Array(ordered.prefix(500)) }
        keys = Set(ordered)
        UserDefaults.standard.set(ordered, forKey: storageKey)
    }
}

struct MagnetSourceBucket: Identifiable {
    let id: String
    let label: String
    var loading: Bool = true
    var ok: Bool = false
    var count: Int = 0
    var elapsedMs: Int64 = 0
    var error: String = ""
    var items: [UniversalMagnetItem] = []
}

struct MagnetSearchView: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var copiedStore = CopiedMagnetsStore.shared
    @AppStorage("pml.magnet.history.v1") private var historyRaw = ""

    @State private var query = ""
    @State private var category = "all"
    @State private var sort = "seeders"
    @State private var errorMsg = ""
    @State private var buckets: [MagnetSourceBucket] = []
    @State private var activeSourceId = ""
    @State private var searchGen = 0
    @State private var javDetail: MovieMetadata?
    @State private var javMagnets: [MovieMagnetItem] = []
    @State private var copiedHash = ""
    @State private var filesTarget: MagnetFilesSheetTarget?

    private let activeSources: [(id: String, label: String)] = [
        ("bt4g", "BT4G 中文"),
        ("dmhy", "动漫花园"),
        ("knaben", "Knaben"),
        ("nyaa", "Nyaa"),
    ]

    private var loading: Bool { buckets.contains(where: { $0.loading }) }
    private var activeBucket: MagnetSourceBucket? {
        buckets.first(where: { $0.id == activeSourceId }) ?? buckets.first
    }

    private let categories: [(id: String, label: String)] = [
        ("all", "全部"),
        ("jav", "番号"),
        ("video", "影视"),
        ("anime", "动漫"),
        ("software", "软件"),
        ("audio_book", "书籍"),
    ]

    private var sortLabel: String {
        switch sort {
        case "size_desc": return "体积"
        case "date_desc": return "最新"
        default: return "做种"
        }
    }

    private var historyList: [String] {
        historyRaw.split(separator: "\n").map(String.init).filter { !$0.isEmpty }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                // 简洁搜索框 + 排序菜单
                HStack(spacing: 8) {
                    TextField("搜索影片、动漫、软件或番号…", text: $query)
                        .textFieldStyle(.roundedBorder)
                        .submitLabel(.search)
                        .onSubmit { Task { await runSearch() } }
                    Button {
                        Task { await runSearch() }
                    } label: {
                        if loading {
                            ProgressView().controlSize(.small)
                        } else {
                            Text("搜索")
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(.blue)
                    .disabled(query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .padding(.horizontal, 14)

                // 单行分类条 + 内联排序
                HStack(spacing: 8) {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(categories, id: \.id) { c in
                                Button {
                                    category = c.id
                                    if !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                        Task { await runSearch() }
                                    }
                                } label: {
                                    Text(c.label)
                                        .font(.caption.weight(.semibold))
                                        .padding(.horizontal, 11)
                                        .padding(.vertical, 6)
                                        .background(category == c.id ? Color.blue : Color.secondary.opacity(0.14), in: Capsule())
                                        .foregroundStyle(category == c.id ? .white : .primary)
                                }
                            }
                        }
                    }
                    Menu {
                        Button("做种最多") { sort = "seeders"; if !query.isEmpty { Task { await runSearch() } } }
                        Button("体积最大") { sort = "size_desc"; if !query.isEmpty { Task { await runSearch() } } }
                        Button("最新发布") { sort = "date_desc"; if !query.isEmpty { Task { await runSearch() } } }
                    } label: {
                        Label(sortLabel, systemImage: "arrow.up.arrow.down")
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 9)
                            .padding(.vertical, 6)
                            .background(Color.secondary.opacity(0.14), in: Capsule())
                            .foregroundStyle(.primary)
                    }
                }
                .padding(.horizontal, 14)

                // 搜索历史（未搜时展示）
                if buckets.isEmpty && !historyList.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Label("最近搜索", systemImage: "clock.arrow.circlepath")
                                .font(.caption.bold())
                                .foregroundStyle(.secondary)
                            Spacer()
                            Button("清空") { historyRaw = "" }
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 8) {
                                ForEach(historyList, id: \.self) { term in
                                    Button(term) {
                                        query = term
                                        Task { await runSearch() }
                                    }
                                    .font(.caption)
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 5)
                                    .background(Color.secondary.opacity(0.14), in: Capsule())
                                    .foregroundStyle(.primary)
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 14)
                }

                if !errorMsg.isEmpty {
                    Text(errorMsg)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .padding(.horizontal, 14)
                }

                // 分源标签栏（先返回先展示，其他源转圈，不同源不混排）
                if !buckets.isEmpty {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(buckets) { b in
                                Button {
                                    activeSourceId = b.id
                                } label: {
                                    HStack(spacing: 5) {
                                        if b.loading {
                                            ProgressView().controlSize(.mini)
                                        } else {
                                            Image(systemName: b.ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                                                .foregroundStyle(b.ok ? .green : .red)
                                        }
                                        Text(b.label)
                                        if !b.loading {
                                            Text("\(b.count)")
                                                .fontWeight(.bold)
                                        }
                                    }
                                    .font(.caption.weight(.semibold))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(
                                        activeBucket?.id == b.id ? Color.blue.opacity(0.22) : Color.secondary.opacity(0.12),
                                        in: Capsule()
                                    )
                                    .overlay(
                                        Capsule().stroke(activeBucket?.id == b.id ? Color.blue : Color.clear, lineWidth: 1)
                                    )
                                    .foregroundStyle(.primary)
                                }
                            }
                        }
                        .padding(.horizontal, 14)
                    }
                }

                if let active = activeBucket {
                    if active.loading {
                        ProgressView("正在检索「\(active.label)」…")
                            .frame(maxWidth: .infinity)
                            .padding(.top, 36)
                    } else if active.items.isEmpty {
                        ContentUnavailableView(
                            "「\(active.label)」暂无匹配磁力",
                            systemImage: "link.badge.plus",
                            description: Text("可点击上方其他数据源切换查看，或缩短关键词重试。")
                        )
                        .padding(.top, 30)
                    } else {
                        LazyVStack(spacing: 10) {
                            ForEach(active.items) { item in
                                VStack(alignment: .leading, spacing: 8) {
                                    HStack(alignment: .top, spacing: 6) {
                                        Text(item.tracker)
                                            .font(.system(size: 10, weight: .bold))
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(trackerColor(item.source).opacity(0.2), in: RoundedRectangle(cornerRadius: 4))
                                            .foregroundStyle(trackerColor(item.source))
                                        Text(item.title)
                                            .font(.subheadline.weight(.semibold))
                                            .foregroundStyle(.primary)
                                            .lineLimit(3)
                                    }

                                    HStack(spacing: 10) {
                                        Text(item.sizeText)
                                            .font(.caption.bold())
                                        Label("\(item.seeders)", systemImage: "arrow.up")
                                            .font(.caption.bold())
                                            .foregroundStyle(.green)
                                        Label("\(item.leechers)", systemImage: "arrow.down")
                                            .font(.caption)
                                            .foregroundStyle(.orange)
                                        if !item.pubDate.isEmpty {
                                            Text(item.pubDate)
                                                .font(.caption2)
                                                .foregroundStyle(.secondary)
                                        }
                                        Spacer()
                                    }

                                    HStack(spacing: 10) {
                                        Button {
                                            UIPasteboard.general.string = item.magnet
                                            copiedHash = item.infoHash
                                            copiedStore.markCopied(item.infoHash)
                                        } label: {
                                            Label(
                                                copiedStore.isCopied(item.infoHash) ? "已复制" : "复制磁力",
                                                systemImage: copiedStore.isCopied(item.infoHash) ? "checkmark" : "doc.on.doc"
                                            )
                                            .font(.caption.bold())
                                        }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.small)
                                        .tint(copiedStore.isCopied(item.infoHash) ? .green : .blue)

                                        Button {
                                            filesTarget = MagnetFilesSheetTarget(infoHash: item.infoHash, title: item.title)
                                        } label: {
                                            Label("文件", systemImage: "folder")
                                                .font(.caption)
                                        }
                                        .buttonStyle(.bordered)
                                        .controlSize(.small)

                                        ShareLink(item: item.magnet) {
                                            Image(systemName: "square.and.arrow.up")
                                                .font(.caption)
                                        }
                                        .buttonStyle(.bordered)
                                        .controlSize(.small)

                                        Spacer()
                                        Text(item.category)
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                            .lineLimit(1)
                                    }
                                }
                                .padding(12)
                                .pmlGlassCard(cornerRadius: 14)
                            }
                        }
                        .padding(.horizontal, 14)
                    }
                } else {
                    ContentUnavailableView(
                        "全网磁力分源搜索",
                        systemImage: "link.circle.fill",
                        description: Text("独立并发检索 BT4G 中文、动漫花园、Knaben 与 Nyaa，先返回先展示。")
                    )
                    .padding(.top, 36)
                }
            }
            .padding(.vertical, 10)
        }
        .navigationTitle("磁力搜索")
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $filesTarget) { t in
            MagnetFilesSheetView(target: t).environmentObject(model)
        }
    }

    private func trackerColor(_ source: String) -> Color {
        switch source {
        case "bt4g": return .purple
        case "dmhy": return .teal
        case "tpb": return .orange
        case "nyaa": return .green
        case "sukebei": return .pink
        default: return .blue
        }
    }

    private func saveHistory(_ term: String) {
        let updated = [term] + historyList.filter { $0 != term }
        historyRaw = updated.prefix(12).joined(separator: "\n")
    }

    private func runSearch() async {
        guard let client = model.apiClient else { return }
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return }
        saveHistory(q)
        searchGen += 1
        let gen = searchGen
        errorMsg = ""
        javDetail = nil
        javMagnets = []
        activeSourceId = ""
        let apiCat = category == "jav" ? "adult" : category
        let targets = activeSources
        buckets = targets.map { MagnetSourceBucket(id: $0.id, label: $0.label) }

        for src in targets {
            Task {
                do {
                    let res = try await client.searchAllMagnets(
                        query: q,
                        category: apiCat,
                        sort: sort,
                        sources: [src.id],
                        limit: 60
                    )
                    await MainActor.run {
                        guard gen == searchGen else { return }
                        if let idx = buckets.firstIndex(where: { $0.id == src.id }) {
                            let st = res.sourceStats.first
                            buckets[idx].loading = false
                            buckets[idx].ok = st?.ok ?? true
                            buckets[idx].count = res.items.count
                            buckets[idx].elapsedMs = st?.elapsedMs ?? 0
                            buckets[idx].error = st?.error ?? ""
                            buckets[idx].items = res.items
                        }
                        if activeSourceId.isEmpty && !res.items.isEmpty {
                            activeSourceId = src.id
                        } else if activeSourceId.isEmpty && buckets.allSatisfy({ !$0.loading }) {
                            activeSourceId = buckets.first?.id ?? ""
                        }
                    }
                } catch {
                    await MainActor.run {
                        guard gen == searchGen else { return }
                        if let idx = buckets.firstIndex(where: { $0.id == src.id }) {
                            buckets[idx].loading = false
                            buckets[idx].ok = false
                            buckets[idx].error = error.localizedDescription
                        }
                        if activeSourceId.isEmpty && buckets.allSatisfy({ !$0.loading }) {
                            activeSourceId = buckets.first?.id ?? ""
                        }
                    }
                }
            }
        }
    }
}
