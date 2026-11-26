import SwiftUI

public struct CleanupView: View {
    @EnvironmentObject private var appModel: AppModel

    @State private var whitelist: [String] = []

    @State private var scanning = false
    @State private var deleting = false
    @State private var cleanupItems: [CleanupItem] = []
    @State private var selectedIds: Set<String> = []
    @State private var selectedCategory: String = "all"
    @State private var showConfirmAlert = false
    @State private var statusMessage: String?

    @State private var batchStats: BatchScrapeStats?
    @State private var batchProgress: BatchScrapeProgress?
    @State private var batchLoading = false

    public init() {}

    private var filteredItems: [CleanupItem] {
        if selectedCategory == "all" { return cleanupItems }
        return cleanupItems.filter { $0.category == selectedCategory }
    }

    public var body: some View {
        Form {
            // 1. 影视库刮削目录（统一收口至「设置 → 五大模块路径与同步」）
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text("刮削目录已与「多模块资料库 → 媒体库」双向同步，请统一在模块设置中管理，避免多处配置冲突。")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    if !whitelist.isEmpty {
                        ForEach(whitelist, id: \.self) { path in
                            HStack {
                                Image(systemName: "folder")
                                    .foregroundStyle(.indigo)
                                Text(path)
                                    .font(.subheadline)
                            }
                            .padding(.vertical, 2)
                        }
                    } else {
                        Text("当前未配置任何影视库目录")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }

                    NavigationLink {
                        LibraryModulesSettingsView()
                    } label: {
                        Label("前往多模块资料库管理目录", systemImage: "square.stack.3d.up.fill")
                            .font(.subheadline.weight(.medium))
                    }
                    .padding(.top, 4)
                }
            } header: {
                Label("影视库刮削目录（已统一收口）", systemImage: "shield.lefthalf.filled")
            }

            // 1.5 全自动后台批量刮削
            Section {
                VStack(alignment: .leading, spacing: 10) {
                    Text("服务端后台单线程温和防封（约2秒间隔）刮削日本影片百科，支持随时暂停、断点续跑。")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    HStack(spacing: 8) {
                        VStack(alignment: .leading) {
                            Text("\(batchStats?.totalWhitelistedVideos ?? 0)")
                                .font(.headline.bold())
                            Text("白名单视频").font(.caption2).foregroundStyle(.secondary)
                        }
                        Spacer()
                        VStack(alignment: .leading) {
                            Text("\(batchStats?.distinctNumbers ?? 0)")
                                .font(.headline.bold())
                            Text("唯一番号").font(.caption2).foregroundStyle(.secondary)
                        }
                        Spacer()
                        VStack(alignment: .leading) {
                            Text("\(batchStats?.alreadyScraped ?? 0)")
                                .font(.headline.bold())
                                .foregroundStyle(.green)
                            Text("已刮削").font(.caption2).foregroundStyle(.secondary)
                        }
                        Spacer()
                        VStack(alignment: .leading) {
                            Text("\(batchStats?.remainingToScrape ?? 0)")
                                .font(.headline.bold())
                                .foregroundStyle(.purple)
                            Text("待入库").font(.caption2).foregroundStyle(.secondary)
                        }
                    }
                    .padding(.vertical, 2)

                    if let prog = batchProgress, prog.running {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text(prog.paused ? "已暂停" : "正在全自动刮削...")
                                    .font(.subheadline.bold())
                                    .foregroundStyle(prog.paused ? .orange : .purple)
                                Spacer()
                                Text("\(prog.completed) / \(prog.total)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            if let cur = prog.currentNumber {
                                Text("当前番号: \(cur)")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                            ProgressView(value: Double(prog.completed), total: Double(max(prog.total, 1)))
                                .tint(prog.paused ? .orange : .purple)
                        }
                    }

                    HStack {
                        if batchProgress?.running != true {
                            Button {
                                Task { await startBatch() }
                            } label: {
                                if batchLoading {
                                    ProgressView()
                                } else {
                                    Label("启动全自动刮削", systemImage: "bolt.fill")
                                }
                            }
                            .buttonStyle(.borderedProminent)
                            .tint(.purple)
                            .controlSize(.small)
                            .disabled(batchLoading)
                        } else {
                            if batchProgress?.paused == true {
                                Button("继续") {
                                    Task { await resumeBatch() }
                                }
                                .buttonStyle(.bordered)
                                .tint(.green)
                                .controlSize(.small)
                            } else {
                                Button("暂停") {
                                    Task { await pauseBatch() }
                                }
                                .buttonStyle(.bordered)
                                .tint(.orange)
                                .controlSize(.small)
                            }

                            Button("终止", role: .destructive) {
                                Task { await stopBatch() }
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                        }
                    }
                }
            } header: {
                Label("全自动后台防封批量刮削", systemImage: "bolt.badge.clock.fill")
            }

            // 2. 垃圾广告扫描与清洗
            Section {
                VStack(alignment: .leading, spacing: 10) {
                    Text("排查离线下载残留的宣传文本（*.txt）、广告链接（*.url）、广告海报图与小于25秒的片头广告切片。")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    HStack {
                        Button {
                            Task { await startScan() }
                        } label: {
                            if scanning {
                                ProgressView()
                            } else {
                                Label("开始排查垃圾文件", systemImage: "magnifyingglass")
                            }
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        .disabled(scanning)

                        if !cleanupItems.isEmpty {
                            Spacer()
                            Button(role: .destructive) {
                                showConfirmAlert = true
                            } label: {
                                Label("清理 (\(selectedIds.count))", systemImage: "trash.fill")
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .disabled(selectedIds.isEmpty || deleting)
                        }
                    }

                    if let msg = statusMessage {
                        Text(msg)
                            .font(.footnote)
                            .foregroundStyle(.green)
                    }
                }

                if !cleanupItems.isEmpty {
                    Picker("分类", selection: $selectedCategory) {
                        Text("全部 (\(cleanupItems.count))").tag("all")
                        Text("文本 (\(countCategory("text")))").tag("text")
                        Text("图片 (\(countCategory("image")))").tag("image")
                        Text("片头视频 (\(countCategory("video")))").tag("video")
                    }
                    .pickerStyle(.segmented)

                    HStack {
                        Button(selectedIds.count == filteredItems.count ? "取消全选" : "全选当前") {
                            if selectedIds.count == filteredItems.count {
                                selectedIds.removeAll()
                            } else {
                                selectedIds = Set(filteredItems.map { $0.assetId })
                            }
                        }
                        .font(.caption)
                        Spacer()
                    }

                    ForEach(filteredItems) { item in
                        HStack(alignment: .top, spacing: 10) {
                            Button {
                                if selectedIds.contains(item.assetId) {
                                    selectedIds.remove(item.assetId)
                                } else {
                                    selectedIds.insert(item.assetId)
                                }
                            } label: {
                                Image(systemName: selectedIds.contains(item.assetId) ? "checkmark.circle.fill" : "circle")
                                    .foregroundStyle(selectedIds.contains(item.assetId) ? .indigo : .secondary)
                                    .font(.title3)
                            }
                            .buttonStyle(.plain)
                            .padding(.top, 2)

                            VStack(alignment: .leading, spacing: 3) {
                                HStack {
                                    Text(item.category == "text" ? "文本" : item.category == "image" ? "图片" : "视频")
                                        .font(.system(size: 9, weight: .bold))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 2)
                                        .background(categoryColor(item.category).opacity(0.18))
                                        .foregroundStyle(categoryColor(item.category))
                                        .clipShape(RoundedRectangle(cornerRadius: 3))

                                    Text(item.fileName)
                                        .font(.subheadline)
                                        .lineLimit(1)
                                }

                                Text(item.reason)
                                    .font(.caption2)
                                    .foregroundStyle(.orange)

                                Text(formatBytes(item.sizeBytes))
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 2)
                    }
                }
            } header: {
                Label("垃圾文件治理与安全清洗", systemImage: "trash")
            }
        }
        .navigationTitle("治理与清理中心")
        .navigationBarTitleDisplayMode(.inline)
        .alert("确认清理", isPresented: $showConfirmAlert) {
            Button("彻底删除", role: .destructive) {
                Task { await executeCleanup() }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("确定要从资料库与存储中彻底清理选中的 \(selectedIds.count) 个文件吗？此操作无法撤销。")
        }
        .task {
            whitelist = (try? await appModel.apiClient?.getScrapeWhitelist()) ?? []
            await loadBatchData()
        }
    }

    private func countCategory(_ cat: String) -> Int {
        cleanupItems.filter { $0.category == cat }.count
    }

    private func categoryColor(_ cat: String) -> Color {
        switch cat {
        case "text": return .orange
        case "image": return .green
        case "video": return .red
        default: return .secondary
        }
    }

    private func startScan() async {
        guard let client = appModel.apiClient else {
            statusMessage = "未连接到数据源"
            return
        }
        scanning = true
        statusMessage = nil
        cleanupItems = []
        selectedIds.removeAll()
        do {
            let items = try await client.scanCleanupItems()
            cleanupItems = items
            selectedIds = Set(items.map { $0.assetId })
            if items.isEmpty {
                statusMessage = "未检测到匹配的牛皮癣文件或广告片头。"
            }
        } catch {
            statusMessage = "扫描失败: \(error.localizedDescription)"
        }
        scanning = false
    }

    private func executeCleanup() async {
        guard let client = appModel.apiClient else {
            statusMessage = "未连接到数据源"
            return
        }
        deleting = true
        statusMessage = nil
        do {
            let report = try await client.deleteCleanupItems(assetIds: Array(selectedIds))
            let deletedCount = report.deletedCount
            let freed = formatBytes(report.freedBytes)
            cleanupItems.removeAll { selectedIds.contains($0.assetId) }
            selectedIds.removeAll()
            statusMessage = "已彻底清理 \(deletedCount) 个文件，释放 \(freed)！"
        } catch {
            statusMessage = "清理失败: \(error.localizedDescription)"
        }
        deleting = false
    }

    private func formatBytes(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    private func loadBatchData() async {
        guard let client = appModel.apiClient else { return }
        if let stats = try? await client.getBatchScrapeStats() {
            self.batchStats = stats
        }
        if let prog = try? await client.getBatchScrapeProgress() {
            self.batchProgress = prog
        }
    }

    private func startBatch() async {
        guard let client = appModel.apiClient else { return }
        batchLoading = true
        do {
            try await client.startBatchScrape()
            await loadBatchData()
        } catch {
            statusMessage = "启动批量刮削失败: \(error.localizedDescription)"
        }
        batchLoading = false
    }

    private func pauseBatch() async {
        guard let client = appModel.apiClient else { return }
        if let prog = try? await client.pauseBatchScrape() {
            self.batchProgress = prog
        }
    }

    private func resumeBatch() async {
        guard let client = appModel.apiClient else { return }
        if let prog = try? await client.resumeBatchScrape() {
            self.batchProgress = prog
        }
    }

    private func stopBatch() async {
        guard let client = appModel.apiClient else { return }
        if let prog = try? await client.stopBatchScrape() {
            self.batchProgress = prog
            await loadBatchData()
        }
    }
}
