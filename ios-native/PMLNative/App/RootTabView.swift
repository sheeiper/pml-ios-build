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
                tile("离线下载", "icloud.and.arrow.down.fill", .magnets)
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
    @State private var mainTab = 0 // 0 = 搜索, 1 = 下载, 2 = 整理
    @State private var cloudStatus: OfflineCloudStatusResponse?
    @State private var cloudLoading = false
    @State private var cloudError = ""
    @State private var cloudPage = 1
    @State private var dlSubTab = 0 // 0 = 正在下载, 1 = 失败, 2 = 完成记录
    @State private var batchResult: BatchOfflineSubmitResponse?
    @State private var inboxScanning = false
    @State private var inboxHasScanned = false
    @State private var inboxError = ""
    @State private var inboxItems: [InboxPlanItem] = []
    @State private var orgBusyPath = ""
    @State private var offlineMovieDir = ""
    @State private var offlineCreatorPendingDir = ""
    @State private var offlineComicDir = ""
    @State private var offlineGalleryDir = ""
    @State private var offlineCollectionPendingDir = ""
    @State private var cleanPreviewTarget: InboxPlanItem?
    @State private var cleanPreviewNewName = ""
    @State private var uncheckedJunkPaths: Set<String> = []
    @State private var showTrashSheet = false
    @State private var trashLoading = false
    @State private var trashError = ""
    @State private var trashData: InboxTrashResponse?
    @State private var trashRestoreDirs: [String: String] = [:]
    @State private var trashBusyKey = ""
    @State private var folderPickerTarget: CloudFolderPickerTarget?
    @State private var showOfflineSheet = false
    @State private var offlineInboxPath = "/115open/PML_Inbox"
    @State private var offlineManualMagnet = ""
    @State private var offlineManualTitle = ""
    @State private var offlineTasks: [OfflineDownloadTask] = []
    @State private var offlinedHashes: Set<String> = []
    @State private var offlineBusyKey = ""
    @State private var offlineMessage = ""
    @State private var offlineError = ""

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
        VStack(spacing: 0) {
            Picker("功能", selection: $mainTab) {
                Text("搜索").tag(0)
                Text("下载").tag(1)
                Text("整理").tag(2)
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .onChange(of: mainTab) { _, newVal in
                if newVal == 1 && cloudStatus == nil && !cloudLoading {
                    Task { await loadCloudStatus(page: 1) }
                }
            }

            if mainTab == 1 {
                cloudDownloadTabView
            } else if mainTab == 2 {
                inboxOrganizeTabView
            } else {
                searchTabView
            }
        }
        .navigationTitle("离线下载")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    batchResult = nil
                    showOfflineSheet = true
                    Task { await loadOfflineState() }
                } label: {
                    Label("手动添加", systemImage: "plus.circle")
                }
            }
        }
        .task { await loadOfflineState() }
        .sheet(item: $filesTarget) { t in
            MagnetFilesSheetView(target: t).environmentObject(model)
        }
        .sheet(isPresented: $showOfflineSheet) {
            offlineSheetView
        }
        .sheet(item: $cleanPreviewTarget) { target in
            cleanPreviewSheetView(target)
        }
        .sheet(isPresented: $showTrashSheet) {
            trashSheetView
        }
    }

    private var offlineSheetView: some View {
            NavigationStack {
                Form {
                    if !offlineError.isEmpty {
                        Section { Text(offlineError).font(.caption).foregroundStyle(.red) }
                    }
                    if !offlineMessage.isEmpty {
                        Section { Text(offlineMessage).font(.caption).foregroundStyle(.green) }
                    }
                    Section("手动粘贴多行链接（每行一条，最多 20 条）") {
                        TextField("magnet:?xt=urn:btih:...\n支持多行批量粘贴", text: $offlineManualMagnet, axis: .vertical)
                            .lineLimit(4...8)
                        Button {
                            Task { await submitManualBatch() }
                        } label: {
                            HStack {
                                if offlineBusyKey == "manual" { ProgressView() }
                                Text("提交到 115 离线队列")
                            }
                        }
                        .disabled(offlineManualMagnet.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !offlineBusyKey.isEmpty)
                    }
                    if let res = batchResult {
                        Section("提交结果（成功 \(res.succeeded) / 失败 \(res.failed)）") {
                            ForEach(res.results) { r in
                                Label(r.message, systemImage: r.ok ? "checkmark.circle.fill" : "xmark.circle.fill")
                                    .font(.caption)
                                    .foregroundStyle(r.ok ? .green : .red)
                            }
                        }
                    }
                    Section("目录路径配置") {
                        dirPickerRow(label: "收件箱", text: $offlineInboxPath, field: .inbox)
                        dirPickerRow(label: "影视库直达", text: $offlineMovieDir, field: .movie)
                        dirPickerRow(label: "待整理 · 创作者", text: $offlineCreatorPendingDir, field: .creator)
                        dirPickerRow(label: "待整理 · 图集", text: $offlineGalleryDir, field: .gallery)
                        dirPickerRow(label: "待整理 · 漫画", text: $offlineComicDir, field: .comic)
                        dirPickerRow(label: "待整理 · 合集", text: $offlineCollectionPendingDir, field: .collection)
                        Button("保存全部目录") { Task { await saveInboxPath() } }
                    }
                }
                .navigationTitle("手动添加离线链接")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) { Button("完成") { showOfflineSheet = false } }
                }
                .sheet(item: $folderPickerTarget) { target in
                    CloudFolderPickerSheet(
                        title: target.title,
                        initialPath: target.initialPath
                    ) { picked in
                        switch target.field {
                        case .inbox: offlineInboxPath = picked
                        case .movie: offlineMovieDir = picked
                        case .creator: offlineCreatorPendingDir = picked
                        case .gallery: offlineGalleryDir = picked
                        case .comic: offlineComicDir = picked
                        case .collection: offlineCollectionPendingDir = picked
                        }
                    }
                    .environmentObject(model)
                }
            }
    }

    private func cleanPreviewSheetView(_ target: InboxPlanItem) -> some View {
            NavigationStack {
                List {
                    Section("主文件夹命名对比") {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("清洗前").font(.caption2).foregroundStyle(.secondary)
                            Text(target.name).font(.caption).foregroundStyle(.secondary)
                        }
                        VStack(alignment: .leading, spacing: 4) {
                            Text("清洗后（可手动编辑）").font(.caption2).foregroundStyle(.green)
                            TextField("规范名称", text: $cleanPreviewNewName)
                                .font(.subheadline)
                        }
                    }

                    Section("清洗前 · 原文件列表（点选可取消误删）") {
                        ForEach(target.keptFiles, id: \.fullPath) { kf in
                            HStack {
                                Image(systemName: "doc").foregroundStyle(.secondary)
                                Text(relativePath(kf.fullPath, root: target.fullPath))
                                    .font(.caption)
                                Spacer()
                                Text(kf.sizeText).font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                        ForEach(target.junkFiles, id: \.fullPath) { jf in
                            junkFileRow(jf, root: target.fullPath)
                        }
                    }

                    Section("清洗后 · 最终目录预览") {
                        let finalTitle = cleanPreviewNewName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            ? target.suggestedName : cleanPreviewNewName.trimmingCharacters(in: .whitespacesAndNewlines)
                        Label("\(finalTitle)/", systemImage: "folder.fill")
                            .font(.caption.bold())
                            .foregroundStyle(.green)
                        let afterList = computeCleanAfterRows(target)
                        ForEach(afterList, id: \.fullPath) { row in
                            HStack {
                                Image(systemName: row.renamed ? "pencil.circle.fill" : "checkmark.circle.fill")
                                    .foregroundStyle(row.renamed ? .blue : .green)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(row.rel).font(.caption)
                                    if row.renamed {
                                        Text("已去除网站水印").font(.caption2).foregroundStyle(.blue)
                                    } else if row.rescued {
                                        Text("手动保留").font(.caption2).foregroundStyle(.yellow)
                                    }
                                }
                                Spacer()
                                Text(row.sizeText).font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                .navigationTitle("清洗前后整体对比")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("取消") { cleanPreviewTarget = nil }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            Task {
                                await runCleanItem(
                                    target,
                                    customName: cleanPreviewNewName,
                                    excludedTrash: uncheckedJunkPaths
                                )
                                cleanPreviewTarget = nil
                            }
                        } label: {
                            if orgBusyPath == target.fullPath {
                                ProgressView().controlSize(.mini)
                            } else {
                                Text("确认清洗").bold()
                            }
                        }
                        .disabled(!orgBusyPath.isEmpty)
                    }
                }
            }
    }

    private func junkFileRow(_ jf: InboxFileEntry, root: String) -> some View {
        let rescued = uncheckedJunkPaths.contains(jf.fullPath)
        return Button {
            if rescued { uncheckedJunkPaths.remove(jf.fullPath) }
            else { uncheckedJunkPaths.insert(jf.fullPath) }
        } label: {
            HStack {
                Image(systemName: rescued ? "square" : "checkmark.square.fill")
                    .foregroundStyle(rescued ? .yellow : .red)
                VStack(alignment: .leading, spacing: 2) {
                    Text(relativePath(jf.fullPath, root: root))
                        .font(.caption)
                        .strikethrough(!rescued)
                        .foregroundStyle(rescued ? .primary : .red)
                    Text(rescued ? "已取消（保留）" : "\(jf.sizeText) · \(jf.reason)")
                        .font(.caption2)
                        .foregroundStyle(rescued ? .yellow : .secondary)
                }
            }
        }
    }

    private var trashSheetView: some View {
            NavigationStack {
                List {
                    if !trashError.isEmpty {
                        Section { Text(trashError).font(.caption).foregroundStyle(.red) }
                    }
                    if trashLoading && trashData == nil {
                        Section {
                            HStack {
                                Spacer()
                                ProgressView("正在读取 _Trash 垃圾箱…")
                                Spacer()
                            }
                        }
                    } else if let td = trashData, td.groups.isEmpty && td.looseItems.isEmpty {
                        Section {
                            ContentUnavailableView("垃圾箱为空", systemImage: "checkmark.circle")
                        }
                    } else if let td = trashData {
                        ForEach(td.groups) { g in
                            Section {
                                TextField(
                                    "还原目标路径",
                                    text: Binding(
                                        get: { trashRestoreDirs[g.groupPath] ?? g.suggestedRestoreDir },
                                        set: { trashRestoreDirs[g.groupPath] = $0 }
                                    )
                                )
                                .font(.caption)
                                ForEach(g.items) { item in
                                    HStack {
                                        Image(systemName: item.isDir ? "folder" : "doc")
                                            .foregroundStyle(.secondary)
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(item.name).font(.caption).lineLimit(2)
                                            Text(item.sizeText).font(.caption2).foregroundStyle(.secondary)
                                        }
                                        Spacer()
                                        Button("还原") {
                                            let dest = trashRestoreDirs[g.groupPath] ?? g.suggestedRestoreDir
                                            Task { await runRestoreTrash(paths: [item.fullPath], targetDir: dest, busyKey: item.fullPath) }
                                        }
                                        .buttonStyle(.bordered)
                                        .controlSize(.mini)
                                        .disabled(!trashBusyKey.isEmpty)
                                    }
                                }
                                Button {
                                    let dest = trashRestoreDirs[g.groupPath] ?? g.suggestedRestoreDir
                                    Task { await runRestoreTrash(paths: g.items.map { $0.fullPath }, targetDir: dest, busyKey: g.groupPath) }
                                } label: {
                                    HStack {
                                        if trashBusyKey == g.groupPath { ProgressView().controlSize(.mini) }
                                        Label("一键还原整组 (\(g.items.count) 项)", systemImage: "arrow.uturn.backward.circle")
                                    }
                                }
                                .disabled(!trashBusyKey.isEmpty)
                            } header: {
                                Text("\(g.groupName) · \(g.restoreDirExists ? "原目录存在" : "自动新建")")
                            }
                        }
                        if !td.looseItems.isEmpty {
                            Section("零散文件") {
                                ForEach(td.looseItems) { item in
                                    HStack {
                                        Text(item.name).font(.caption)
                                        Spacer()
                                        Button("还原") {
                                            Task { await runRestoreTrash(paths: [item.fullPath], targetDir: td.inboxPath, busyKey: item.fullPath) }
                                        }
                                        .buttonStyle(.bordered)
                                        .controlSize(.mini)
                                        .disabled(!trashBusyKey.isEmpty)
                                    }
                                }
                            }
                        }
                    }
                }
                .navigationTitle("回收站 (_Trash)")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("刷新") { Task { await loadTrash() } }
                            .disabled(trashLoading)
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("完成") { showTrashSheet = false }
                    }
                }
            }
    }

    private var searchTabView: some View {
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
                                            Task { await submitOffline(magnet: item.magnet, title: item.title, key: item.infoHash) }
                                        } label: {
                                            if offlineBusyKey == item.infoHash {
                                                ProgressView().controlSize(.mini)
                                            } else {
                                                Label(
                                                    offlinedHashes.contains(item.infoHash.lowercased()) ? "已离线" : "离线",
                                                    systemImage: offlinedHashes.contains(item.infoHash.lowercased()) ? "checkmark.icloud.fill" : "icloud.and.arrow.down"
                                                )
                                                .font(.caption.bold())
                                            }
                                        }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.small)
                                        .tint(offlinedHashes.contains(item.infoHash.lowercased()) ? .green : .blue)
                                        .disabled(!offlineBusyKey.isEmpty)

                                        Button {
                                            UIPasteboard.general.string = item.magnet
                                            copiedHash = item.infoHash
                                            copiedStore.markCopied(item.infoHash)
                                        } label: {
                                            Label(
                                                copiedStore.isCopied(item.infoHash) ? "已复制" : "复制",
                                                systemImage: copiedStore.isCopied(item.infoHash) ? "checkmark" : "doc.on.doc"
                                            )
                                            .font(.caption.bold())
                                        }
                                        .buttonStyle(.bordered)
                                        .controlSize(.small)
                                        .tint(copiedStore.isCopied(item.infoHash) ? .green : .secondary)

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
    }

    private var cloudDownloadTabView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                // 配额卡片
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text("云下载配额").font(.subheadline.bold())
                        Spacer()
                        Button {
                            Task { await loadCloudStatus(page: cloudPage) }
                        } label: {
                            if cloudLoading {
                                ProgressView().controlSize(.mini)
                            } else {
                                Image(systemName: "arrow.clockwise")
                            }
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                    }
                    let used = cloudStatus?.quota.used ?? 0
                    let total = max(1, cloudStatus?.quota.total ?? 1)
                    ProgressView(value: Double(min(used, total)), total: Double(total))
                        .tint(.blue)
                    HStack {
                        if let st = cloudStatus {
                            Text("已使用 \(st.quota.used) / 总配额 \(st.quota.total)")
                                .font(.caption2).foregroundStyle(.secondary)
                            Spacer()
                            Text("任务总数: \(st.totalCount)")
                                .font(.caption2).foregroundStyle(.secondary)
                        } else {
                            Text("点击刷新读取 115 云下载配额…")
                                .font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                }
                .padding(12)
                .pmlGlassCard(cornerRadius: 14)

                if !cloudError.isEmpty {
                    Text(cloudError).font(.caption).foregroundStyle(.red)
                }

                let allItems = cloudStatus?.items ?? []
                let dlItems = allItems.filter { $0.statusCategory == "downloading" }
                let failItems = allItems.filter { $0.statusCategory == "failed" }
                let doneItems = allItems.filter { $0.statusCategory == "completed" }
                let shown = dlSubTab == 0 ? dlItems : (dlSubTab == 1 ? failItems : doneItems)

                Picker("状态", selection: $dlSubTab) {
                    Text("正在下载 (\(dlItems.count))").tag(0)
                    Text("失败 (\(failItems.count))").tag(1)
                    Text("完成 (\(cloudStatus?.totalCount ?? Int64(doneItems.count)))").tag(2)
                }
                .pickerStyle(.segmented)

                if cloudLoading && cloudStatus == nil {
                    ProgressView("加载 115 云下载任务…")
                        .frame(maxWidth: .infinity)
                        .padding(.top, 36)
                } else if shown.isEmpty {
                    ContentUnavailableView("暂无离线任务", systemImage: "tray")
                        .padding(.top, 24)
                } else {
                    LazyVStack(spacing: 8) {
                        ForEach(shown) { it in
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(alignment: .top) {
                                    Text(it.name)
                                        .font(.caption.weight(.semibold))
                                        .lineLimit(2)
                                    Spacer()
                                    Text(it.statusCategory == "completed" ? "已完成" : (it.statusCategory == "downloading" ? "\(Int(it.percentDone))%" : "失败"))
                                        .font(.caption2.bold())
                                        .foregroundStyle(it.statusCategory == "completed" ? .green : (it.statusCategory == "downloading" ? .blue : .red))
                                }
                                ProgressView(value: it.percentDone, total: 100)
                                    .tint(it.statusCategory == "completed" ? .green : (it.statusCategory == "downloading" ? .blue : .red))
                                Text(it.sizeText)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                            .padding(10)
                            .pmlGlassCard(cornerRadius: 12)
                        }
                    }
                }

                if let st = cloudStatus, st.pageCount > 1 {
                    HStack {
                        Button("上一页") { Task { await loadCloudStatus(page: max(1, cloudPage - 1)) } }
                            .buttonStyle(.bordered).controlSize(.small)
                            .disabled(cloudPage <= 1 || cloudLoading)
                        Spacer()
                        Text("第 \(cloudPage) / \(st.pageCount) 页").font(.caption2).foregroundStyle(.secondary)
                        Spacer()
                        Button("下一页") { Task { await loadCloudStatus(page: min(st.pageCount, cloudPage + 1)) } }
                            .buttonStyle(.bordered).controlSize(.small)
                            .disabled(cloudPage >= st.pageCount || cloudLoading)
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
        }
    }

    private var inboxOrganizeTabView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("收件箱整理").font(.subheadline.bold())
                        Text(offlineInboxPath).font(.caption2).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button {
                        showTrashSheet = true
                        Task { await loadTrash() }
                    } label: {
                        Image(systemName: "trash")
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                    Button {
                        Task { await runScanInbox() }
                    } label: {
                        if inboxScanning {
                            ProgressView().controlSize(.mini)
                        } else {
                            Label("扫描收件箱", systemImage: "magnifyingglass")
                                .font(.caption.bold())
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(inboxScanning)
                }
                .padding(12)
                .pmlGlassCard(cornerRadius: 14)

                if !inboxError.isEmpty {
                    Text(inboxError).font(.caption).foregroundStyle(.red)
                }

                if !inboxHasScanned && !inboxScanning {
                    ContentUnavailableView("手动扫描收件箱", systemImage: "folder.badge.gearshape", description: Text("点击上方「扫描收件箱」安全预览待清洗与待归档项目"))
                        .padding(.top, 24)
                } else if inboxItems.isEmpty && !inboxScanning {
                    ContentUnavailableView("收件箱已整理干净", systemImage: "checkmark.circle")
                        .padding(.top, 24)
                } else {
                    let pending = inboxItems.filter { $0.needsClean }
                    let ready = inboxItems.filter { !$0.needsClean }

                    if !pending.isEmpty {
                        Text("① 待清洗 (\(pending.count))").font(.caption.bold()).foregroundStyle(.secondary)
                        ForEach(pending) { it in
                            VStack(alignment: .leading, spacing: 8) {
                                Text(it.name).font(.subheadline.weight(.semibold)).lineLimit(2)
                                Text("规范名称 → \(it.suggestedName)").font(.caption).foregroundStyle(.blue)
                                Text("保留 \(it.keptFiles.count) 项（\(it.totalSizeText)） · 移入 _Trash \(it.junkFiles.count) 项")
                                    .font(.caption2).foregroundStyle(.secondary)
                                Button {
                                    cleanPreviewNewName = it.suggestedName
                                    uncheckedJunkPaths = []
                                    cleanPreviewTarget = it
                                } label: {
                                    if orgBusyPath == it.fullPath {
                                        ProgressView().controlSize(.mini)
                                    } else {
                                        Label("对比并清洗", systemImage: "sparkles")
                                            .font(.caption.bold())
                                            .frame(maxWidth: .infinity)
                                    }
                                }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                                .disabled(!orgBusyPath.isEmpty)
                            }
                            .padding(12)
                            .pmlGlassCard(cornerRadius: 12)
                        }
                    }

                    if !ready.isEmpty {
                        Text("② 已清洗 · 待归档 (\(ready.count))").font(.caption.bold()).foregroundStyle(.secondary)
                        ForEach(ready) { it in
                            VStack(alignment: .leading, spacing: 8) {
                                HStack {
                                    Text(it.name).font(.subheadline.weight(.semibold)).lineLimit(2)
                                    Spacer()
                                    Text("已清洗").font(.caption2.bold()).foregroundStyle(.green)
                                }
                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 6) {
                                        Button("影视库") { Task { await runArchiveItem(it, cat: "movie") } }
                                            .buttonStyle(.bordered).controlSize(.small)
                                        Button("创作者") { Task { await runArchiveItem(it, cat: "creator") } }
                                            .buttonStyle(.bordered).controlSize(.small)
                                        Button("图集") { Task { await runArchiveItem(it, cat: "gallery") } }
                                            .buttonStyle(.bordered).controlSize(.small)
                                        Button("漫画") { Task { await runArchiveItem(it, cat: "comic") } }
                                            .buttonStyle(.bordered).controlSize(.small)
                                        Button("合集") { Task { await runArchiveItem(it, cat: "collection") } }
                                            .buttonStyle(.bordered).controlSize(.small)
                                    }
                                }
                                .disabled(!orgBusyPath.isEmpty)
                            }
                            .padding(12)
                            .pmlGlassCard(cornerRadius: 12)
                        }
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
        }
    }

    private func dirPickerRow(label: String, text: Binding<String>, field: CloudFolderPickerField) -> some View {
        HStack(spacing: 8) {
            TextField(label, text: text)
                .font(.caption)
            Button("选择") {
                folderPickerTarget = CloudFolderPickerTarget(
                    field: field,
                    title: "选择\(label)目录",
                    initialPath: text.wrappedValue.isEmpty ? "/115open" : text.wrappedValue
                )
            }
            .buttonStyle(.bordered)
            .controlSize(.mini)
        }
    }

    private func relativePath(_ fullPath: String, root: String) -> String {
        let prefix = root.hasSuffix("/") ? root : "\(root)/"
        if fullPath.hasPrefix(prefix) {
            return String(fullPath.dropFirst(prefix.count))
        }
        return fullPath.split(separator: "/").last.map(String.init) ?? fullPath
    }

    private func computeCleanAfterRows(_ it: InboxPlanItem) -> [(fullPath: String, rel: String, sizeText: String, renamed: Bool, rescued: Bool)] {
        var renameMap: [String: String] = [:]
        for r in (it.fileRenames ?? []) {
            renameMap[r.fullPath] = r.newName
        }
        let flatPrefix: String
        if let src = it.flattenSourceDir, !src.isEmpty {
            flatPrefix = src.hasSuffix("/") ? src : "\(src)/"
        } else {
            flatPrefix = ""
        }

        func mapPath(_ fp: String) -> (String, Bool) {
            var rel = (!flatPrefix.isEmpty && fp.hasPrefix(flatPrefix))
                ? String(fp.dropFirst(flatPrefix.count))
                : relativePath(fp, root: it.fullPath)
            if let nn = renameMap[fp] {
                if let idx = rel.lastIndex(of: "/") {
                    rel = String(rel[...idx]) + nn
                } else {
                    rel = nn
                }
                return (rel, true)
            }
            return (rel, false)
        }

        var rows: [(String, String, String, Bool, Bool)] = []
        for f in it.keptFiles {
            let (rel, renamed) = mapPath(f.fullPath)
            rows.append((f.fullPath, rel, f.sizeText, renamed, false))
        }
        for f in it.junkFiles where uncheckedJunkPaths.contains(f.fullPath) {
            let (rel, renamed) = mapPath(f.fullPath)
            rows.append((f.fullPath, rel, f.sizeText, renamed, true))
        }
        return rows
    }

    private func loadTrash() async {
        guard let client = model.apiClient else { return }
        trashLoading = true
        trashError = ""
        defer { trashLoading = false }
        do {
            let res = try await client.listInboxTrash()
            trashData = res
            var dirs: [String: String] = [:]
            for g in res.groups { dirs[g.groupPath] = g.suggestedRestoreDir }
            trashRestoreDirs = dirs
        } catch {
            trashError = error.localizedDescription
        }
    }

    private func runRestoreTrash(paths: [String], targetDir: String, busyKey: String) async {
        guard let client = model.apiClient, !paths.isEmpty else { return }
        trashBusyKey = busyKey
        trashError = ""
        defer { trashBusyKey = "" }
        do {
            let res = try await client.restoreInboxTrash(paths: paths, targetDir: targetDir)
            offlineMessage = res.message
            await loadTrash()
            if inboxHasScanned {
                await runScanInbox()
            }
        } catch {
            trashError = error.localizedDescription
        }
    }

    private func runScanInbox() async {
        guard let client = model.apiClient else { return }
        inboxScanning = true
        inboxError = ""
        defer { inboxScanning = false }
        do {
            let res = try await client.scanOfflineInbox()
            inboxItems = res.items
            inboxHasScanned = true
            offlineInboxPath = res.config.inboxPath
            offlineMovieDir = res.config.movieDir
            offlineCreatorPendingDir = res.config.creatorPendingDir ?? ""
            offlineComicDir = res.config.comicPendingDir ?? res.config.comicDir
            offlineGalleryDir = res.config.galleryPendingDir ?? res.config.galleryDir
            offlineCollectionPendingDir = res.config.collectionPendingDir ?? ""
        } catch {
            inboxError = error.localizedDescription
        }
    }

    private func runCleanItem(
        _ it: InboxPlanItem,
        customName: String? = nil,
        excludedTrash: Set<String> = []
    ) async {
        guard let client = model.apiClient else { return }
        orgBusyPath = it.fullPath
        inboxError = ""
        defer { orgBusyPath = "" }
        let selectedTrash = it.junkFiles
            .map { $0.fullPath }
            .filter { !excludedTrash.contains($0) }
        let targetName = (customName ?? it.suggestedName).trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let res = try await client.executeInboxClean(
                itemPath: it.fullPath,
                trashPaths: selectedTrash,
                fileRenames: it.fileRenames,
                flattenSourceDir: it.flattenSourceDir,
                flattenWrapperDir: it.flattenWrapperDir,
                newName: (!targetName.isEmpty && targetName != it.name) ? targetName : nil
            )
            if let idx = inboxItems.firstIndex(where: { $0.fullPath == it.fullPath }) {
                inboxItems[idx].fullPath = res.newPath
                if !targetName.isEmpty { inboxItems[idx].name = targetName }
                inboxItems[idx].junkFiles = it.junkFiles.filter { !selectedTrash.contains($0.fullPath) }
                inboxItems[idx].fileRenames = []
                inboxItems[idx].needsClean = false
            }
        } catch {
            inboxError = error.localizedDescription
        }
    }

    private func runArchiveItem(_ it: InboxPlanItem, cat: String) async {
        guard let client = model.apiClient else { return }
        orgBusyPath = it.fullPath
        inboxError = ""
        defer { orgBusyPath = "" }
        do {
            _ = try await client.archiveInboxItem(itemPath: it.fullPath, category: cat)
            inboxItems.removeAll { $0.fullPath == it.fullPath }
        } catch {
            inboxError = error.localizedDescription
        }
    }

    private func loadCloudStatus(page: Int) async {
        guard let client = model.apiClient else { return }
        cloudLoading = true
        cloudError = ""
        defer { cloudLoading = false }
        do {
            let res = try await client.getOfflineCloudStatus(page: page)
            cloudStatus = res
            cloudPage = res.page
        } catch {
            cloudError = error.localizedDescription
        }
    }

    private func submitManualBatch() async {
        guard let client = model.apiClient else { return }
        offlineBusyKey = "manual"
        offlineError = ""
        offlineMessage = ""
        defer { offlineBusyKey = "" }
        do {
            let res = try await client.submitOfflineMagnetsBatch(urlsText: offlineManualMagnet, targetDir: offlineInboxPath)
            batchResult = res
            offlineMessage = "批量提交完成：成功 \(res.succeeded) 条，失败 \(res.failed) 条"
            if res.failed == 0 { offlineManualMagnet = "" }
            await loadCloudStatus(page: 1)
        } catch {
            offlineError = error.localizedDescription
        }
    }

    private func loadOfflineState() async {
        guard let client = model.apiClient else { return }
        if let cfg = try? await client.getOfflineDownloadConfig() {
            offlineInboxPath = cfg.inboxPath
            offlineMovieDir = cfg.movieDir
            offlineCreatorPendingDir = cfg.creatorPendingDir ?? ""
            offlineComicDir = cfg.comicPendingDir ?? cfg.comicDir
            offlineGalleryDir = cfg.galleryPendingDir ?? cfg.galleryDir
            offlineCollectionPendingDir = cfg.collectionPendingDir ?? ""
        }
        if let list = try? await client.listOfflineDownloadTasks(limit: 30) {
            offlineTasks = list
            offlinedHashes = Set(list.flatMap { [$0.infoHash.lowercased(), $0.magnet.lowercased()] }.filter { !$0.isEmpty })
        }
    }

    private func saveInboxPath() async {
        guard let client = model.apiClient else { return }
        offlineError = ""
        do {
            let cfg = try await client.saveOfflineDownloadConfig(
                inboxPath: offlineInboxPath,
                movieDir: offlineMovieDir,
                comicDir: offlineComicDir,
                galleryDir: offlineGalleryDir,
                creatorPendingDir: offlineCreatorPendingDir,
                galleryPendingDir: offlineGalleryDir,
                comicPendingDir: offlineComicDir,
                collectionPendingDir: offlineCollectionPendingDir
            )
            offlineInboxPath = cfg.inboxPath
            offlineMovieDir = cfg.movieDir
            offlineCreatorPendingDir = cfg.creatorPendingDir ?? ""
            offlineComicDir = cfg.comicPendingDir ?? cfg.comicDir
            offlineGalleryDir = cfg.galleryPendingDir ?? cfg.galleryDir
            offlineCollectionPendingDir = cfg.collectionPendingDir ?? ""
            offlineMessage = "目录已保存"
        } catch {
            offlineError = error.localizedDescription
        }
    }

    private func submitOffline(magnet: String, title: String?, key: String) async {
        guard let client = model.apiClient else { return }
        let clean = magnet.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        offlineBusyKey = key
        offlineError = ""
        offlineMessage = ""
        defer { offlineBusyKey = "" }
        do {
            let task = try await client.submitOfflineMagnet(magnet: clean, title: title, targetDir: offlineInboxPath)
            offlineTasks.insert(task, at: 0)
            if !task.infoHash.isEmpty { offlinedHashes.insert(task.infoHash.lowercased()) }
            offlinedHashes.insert(clean.lowercased())
            offlineMessage = "已离线到 \(task.targetDir)"
            if key == "manual" {
                offlineManualMagnet = ""
                offlineManualTitle = ""
            }
        } catch {
            offlineError = error.localizedDescription
            showOfflineSheet = true
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
                            buckets[idx].elapsedMs = Int64(st?.elapsedMs ?? 0)
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


private enum CloudFolderPickerField: String {
    case inbox, movie, creator, gallery, comic, collection
}

private struct CloudFolderPickerTarget: Identifiable {
    let field: CloudFolderPickerField
    let title: String
    let initialPath: String
    var id: String { field.rawValue }
}

private struct CloudFolderPickerSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let title: String
    let initialPath: String
    let onSelect: (String) -> Void

    @State private var currentPath = "/115open"
    @State private var parentPath: String?
    @State private var folders: [Cd2FolderEntry] = []
    @State private var loading = false
    @State private var error = ""
    @State private var showNewAlert = false
    @State private var newFolderName = ""

    var body: some View {
        NavigationStack {
            List {
                Section {
                    HStack {
                        Text(currentPath)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                        Spacer()
                        if let parent = parentPath {
                            Button {
                                Task { await loadDir(parent) }
                            } label: {
                                Label("上一级", systemImage: "arrow.up")
                                    .font(.caption)
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.mini)
                            .disabled(loading)
                        }
                    }
                }

                if !error.isEmpty {
                    Section { Text(error).font(.caption).foregroundStyle(.red) }
                }

                Section("子文件夹") {
                    if loading {
                        HStack {
                            Spacer()
                            ProgressView("正在读取目录…")
                            Spacer()
                        }
                    } else if folders.isEmpty {
                        Text("当前目录下没有子文件夹")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(folders) { f in
                            Button {
                                Task { await loadDir(f.fullPath) }
                            } label: {
                                HStack {
                                    Image(systemName: "folder.fill")
                                        .foregroundStyle(.yellow)
                                    Text(f.name)
                                        .font(.subheadline)
                                        .foregroundStyle(.primary)
                                    Spacer()
                                    Image(systemName: "chevron.right")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { dismiss() }
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button {
                        newFolderName = ""
                        showNewAlert = true
                    } label: {
                        Image(systemName: "folder.badge.plus")
                    }
                    .disabled(loading)

                    Button("选择") {
                        onSelect(currentPath)
                        dismiss()
                    }
                    .bold()
                    .disabled(loading || currentPath == "/")
                }
            }
            .alert("新建文件夹", isPresented: $showNewAlert) {
                TextField("文件夹名称", text: $newFolderName)
                Button("取消", role: .cancel) {}
                Button("创建") {
                    Task { await createSubfolder() }
                }
            }
            .task {
                await loadDir(initialPath.isEmpty ? "/115open" : initialPath)
            }
        }
    }

    private func loadDir(_ path: String) async {
        guard let client = model.apiClient else { return }
        loading = true
        error = ""
        defer { loading = false }
        do {
            let res = try await client.listCd2Folders(path: path)
            currentPath = res.currentPath
            parentPath = res.parentPath
            folders = res.folders
        } catch {
            if path != "/115open" && path != "/" {
                if let fallback = try? await client.listCd2Folders(path: "/115open") {
                    currentPath = fallback.currentPath
                    parentPath = fallback.parentPath
                    folders = fallback.folders
                    return
                }
            }
            self.error = error.localizedDescription
        }
    }

    private func createSubfolder() async {
        let name = newFolderName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, let client = model.apiClient else { return }
        loading = true
        error = ""
        defer { loading = false }
        do {
            let created = try await client.createCd2Folder(parentPath: currentPath, name: name)
            await loadDir(created.fullPath)
        } catch {
            self.error = error.localizedDescription
        }
    }
}
