import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel

    @State private var status: LibraryStatus?
    @State private var cache: CacheStats?
    @State private var roots: [LibraryRoot] = []
    @State private var showAddRoot = false
    @State private var accel = AlistAccelConfig(
        enabled: false,
        apiUrl: "",
        username: "",
        password: nil,
        cd2Prefix: "/115open",
        alistPrefix: "/115open"
    )
    @State private var accelMessage = ""
    @State private var cacheMessage = ""
    @State private var historyMessage = ""
    @State private var rootMessage = ""
    @State private var scanning = false
    @State private var scanPercent: Double = 0
    @State private var scanText = ""
    @State private var showSetup = false
    @State private var selfCheckMessage = ""

    var body: some View {
        Form {
            Section("运行模式") {
                NavigationLink {
                    RuntimeModeView(settings: model.settings)
                } label: {
                    LabeledContent("当前模式", value: model.settings.mode.title)
                }
            }
            mediaSourceLink
            scanSection
            cacheSection
            historySection
            diagnosticSection
            aboutSection
        }
        .navigationTitle("设置")
        .task { await load() }
        .sheet(isPresented: $showSetup, onDismiss: { Task { await load() } }) {
            MediaSourceSetupView()
                .environmentObject(model)
        }
        .sheet(isPresented: $showAddRoot, onDismiss: { Task { await load() } }) {
            AddRootView {
                Task { await load() }
            }
            .environmentObject(model)
        }
    }

    private var mediaSourceLink: some View {
        Section("媒体源") {
            if model.settings.mode == .server {
                VStack(alignment: .leading, spacing: 3) {
                    Text("服务器模式")
                    Text("CD2 索引由服务器管理，App 无需配置")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else {
                NavigationLink {
                    MediaSourcesView()
                } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(status?.connected == true ? "CD2 已连接" : "CD2 未连接")
                        Text(status?.endpoint ?? "尚未配置 CD2 / AList")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
            }
            if let status, !status.connected, status.configured {
                Text(status.message).font(.caption).foregroundStyle(.orange)
            }
        }
    }

    private var diagnosticSection: some View {
        Section("诊断") {
            LabeledContent("内置内核", value: model.coreStatusText)
            LabeledContent("当前数据源", value: "内置内核")
            Button {
                Task { await runSelfCheck() }
            } label: {
                Label("运行自检", systemImage: "waveform.path.ecg")
            }
            if !selfCheckMessage.isEmpty {
                Text(selfCheckMessage)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func runSelfCheck() async {
        guard let client = model.apiClient else {
            selfCheckMessage = model.coreError.map { "内核未启动：\($0)" } ?? "未配置数据源"
            return
        }
        do {
            let status = try await client.health()
            let roots = (try? await client.libraryRoots()) ?? []
            selfCheckMessage = "✅ \(status) · 根目录 \(roots.count) 个 · \(client.configuration.baseURL.absoluteString)"
        } catch {
            selfCheckMessage = "❌ \(error.localizedDescription)"
        }
    }

    private var mediaSourceSection: some View {
        Section("媒体源") {
            if let status {
                LabeledContent("状态", value: status.connected ? "已连接" : "未连接")
                if let endpoint = status.endpoint, !endpoint.isEmpty {
                    LabeledContent("地址", value: endpoint)
                }
                Text(status.message).font(.caption).foregroundStyle(.secondary)
            } else {
                Text("尚未配置").foregroundStyle(.secondary)
            }
            Button("配置媒体源") { showSetup = true }
        }
    }

    private var rootsSection: some View {
        Section {
            if roots.isEmpty {
                Text(roots.isEmpty ? "还没有添加资料库根目录" : "")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(roots) { root in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(root.remotePath)
                            .lineLimit(1)
                        Text(root.kindLabel)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .swipeActions(edge: .trailing) {
                        Button(role: .destructive) {
                            Task { await removeRoot(root) }
                        } label: {
                            Label("删除", systemImage: "trash")
                        }
                    }
                }
            }
            Button {
                showAddRoot = true
            } label: {
                Label("添加根目录", systemImage: "plus.circle")
            }
            .disabled(!(status?.connected ?? false))

            if !rootMessage.isEmpty {
                Text(rootMessage).font(.caption).foregroundStyle(.secondary)
            }
        } header: {
            Text("资料库根目录")
        } footer: {
            Text("综合媒体只索引图片/视频；漫画索引图片目录与 CBZ/ZIP。左滑可删除根目录。")
        }
    }

    private func removeRoot(_ root: LibraryRoot) async {
        guard let client = model.apiClient else { return }
        do {
            try await client.removeLibraryRoot(rootId: root.id)
            rootMessage = "已删除 \(root.remotePath)"
            await load()
        } catch {
            rootMessage = error.localizedDescription
        }
    }

    private var scanSection: some View {
        Section("资料库扫描") {
            Button {
                Task { await scan() }
            } label: {
                Label(scanning ? "扫描中…" : "立即扫描资料库", systemImage: "arrow.triangle.2.circlepath")
            }
            .disabled(scanning)

            if scanning {
                ProgressView(value: scanPercent, total: 100)
                Text(scanText).font(.caption).foregroundStyle(.secondary)
            } else if !scanText.isEmpty {
                Text(scanText).font(.caption).foregroundStyle(.secondary)
            }

            if let cache, let last = cache.lastScanTime {
                LabeledContent("上次扫描", value: Date(timeIntervalSince1970: TimeInterval(last)).formatted(date: .abbreviated, time: .shortened))
            }
        }
    }

    private var accelSection: some View {
        Section {
            Toggle("启用 115 加速", isOn: $accel.enabled)
            TextField("AList 地址（http://IP:5244）", text: $accel.apiUrl)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
            TextField("账号", text: $accel.username)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            SecureField("密码", text: accelPasswordBinding)
            TextField("CD2 路径前缀", text: $accel.cd2Prefix)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            TextField("AList 路径前缀", text: $accel.alistPrefix)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()

            Button("保存加速配置") { Task { await saveAccel() } }
            Button("测试加速连接") { Task { await testAccel() } }

            if !accelMessage.isEmpty {
                Text(accelMessage).font(.caption).foregroundStyle(.secondary)
            }
        } header: {
            Text("115 直连加速")
        } footer: {
            Text("配置后视频会优先走 AList 获取的 115 直链，避开跨境带宽。未启用时自动使用 CD2 通道。")
        }
    }

    private var cacheSection: some View {
        Section("缓存") {
            if let cache {
                LabeledContent("封面缓存", value: pmlFormatBytes(cache.thumbnailBytes))
                LabeledContent("漫画缓存", value: pmlFormatBytes(cache.comicBytes))
                LabeledContent("数据库", value: pmlFormatBytes(cache.databaseBytes))
                LabeledContent("短视频缓存", value: pmlFormatBytes(cache.shortVideoCacheBytes))
                LabeledContent("已索引", value: "\(cache.totalIndexedFiles) 文件 / \(cache.totalIndexedDirectories) 目录")
            } else {
                Text("统计加载中…").foregroundStyle(.secondary)
            }
            Button("清理封面缓存") { Task { await clear(target: "thumbnails") } }
            Button("清理漫画缓存") { Task { await clear(target: "comics") } }
            Button("清理短视频缓存") { Task { await clear(target: "shortVideoFiles") } }

            if !cacheMessage.isEmpty {
                Text(cacheMessage).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var historySection: some View {
        Section("播放") {
            Button("清除播放记录", role: .destructive) {
                Task {
                    guard let client = model.apiClient else { return }
                    try? await client.clearPlaybackHistory()
                    historyMessage = "播放记录已清除"
                }
            }
            if !historyMessage.isEmpty {
                Text(historyMessage).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var aboutSection: some View {
        Section("关于") {
            LabeledContent("版本", value: "2.0.0")
            LabeledContent("内置内核", value: model.coreStatusText)
        }
    }

    private var accelPasswordBinding: Binding<String> {
        Binding(
            get: { accel.password ?? "" },
            set: { accel.password = $0.isEmpty ? nil : $0 }
        )
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        status = try? await client.libraryStatus()
        cache = try? await client.cacheStats()
        roots = (try? await client.libraryRoots()) ?? []
        if let value = try? await client.alistAccelConfig() {
            accel = value
        }
    }

    private func scan() async {
        guard let client = model.apiClient, !scanning else { return }
        scanning = true
        scanPercent = 0
        scanText = "开始扫描…"
        defer { scanning = false }
        do {
            try await client.startScan()
            scanText = "扫描中…"
            for _ in 0..<900 {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard let progress = try? await client.scanProgress() else { break }
                scanPercent = progress.percent
                scanText = "\(progress.message) \(Int(progress.percent))%"
                if !progress.running {
                    scanText = progress.error ?? "扫描完成：索引 \(progress.filesIndexed) 项"
                    break
                }
            }
            await load()
        } catch {
            scanText = error.localizedDescription
        }
    }

    private func saveAccel() async {
        guard let client = model.apiClient else { return }
        do {
            try await client.saveAlistAccelConfig(accel)
            accelMessage = "115 加速配置已保存"
        } catch {
            accelMessage = error.localizedDescription
        }
    }

    private func testAccel() async {
        guard let client = model.apiClient else { return }
        do {
            let result = try await client.testAlistAccel(accel)
            accelMessage = result.ok ? "加速连接正常：\(result.message)" : "加速连接失败：\(result.message)"
        } catch {
            accelMessage = error.localizedDescription
        }
    }

    private func clear(target: String) async {
        guard let client = model.apiClient else { return }
        do {
            try await client.clearCache(target: target)
            cacheMessage = "缓存已清理"
            await load()
        } catch {
            cacheMessage = error.localizedDescription
        }
    }
}
