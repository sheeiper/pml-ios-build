import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: AppModel

    @State private var cache: CacheStats?
    @State private var accel = AlistAccelConfig(
        enabled: false,
        apiUrl: "http://103.119.1.41:45112",
        username: "admin",
        password: nil,
        cd2Prefix: "/115open",
        alistPrefix: "/115"
    )
    @State private var accelMessage = ""
    @State private var cacheMessage = ""
    @State private var historyMessage = ""
    @State private var selfCheckMessage = ""
    @State private var checkingServer = false
    @AppStorage("pml.privacy.shield") private var privacyShieldEnabled = true

    var body: some View {
        Form {
            Section("服务器连接") {
                NavigationLink {
                    RuntimeModeView(settings: model.settings)
                } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        HStack {
                            Text("服务器与直连配置")
                            Spacer()
                            Text(model.isConfigured ? "已配置" : "未配置")
                                .font(.caption)
                                .foregroundStyle(model.isConfigured ? .green : .orange)
                        }
                        Text(model.settings.serverURL)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
            }

            Section("多模块资料库") {
                NavigationLink {
                    LibraryModulesSettingsView()
                } label: {
                    Label("五大模块路径与同步", systemImage: "square.stack.3d.up.fill")
                }
                NavigationLink {
                    ScanCenterView()
                } label: {
                    Label("扫描与索引中心", systemImage: "arrow.triangle.2.circlepath")
                }
            }

            Section("隐私安全") {
                Toggle("多任务后台防窥遮罩", isOn: $privacyShieldEnabled)
                Text("开启后，切入系统多任务卡片或锁屏时自动以模糊保护层遮盖敏感画面，防止窥屏。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            accelSection
            cleanupSection
            cacheSection
            historySection
            diagnosticSection
            aboutSection
        }
        .navigationTitle("设置")
        .task { await load() }
    }

    private var cleanupSection: some View {
        Section("治理与整理") {
            NavigationLink {
                CleanupView()
                    .environmentObject(model)
            } label: {
                Label("治理与清理中心", systemImage: "shield.lefthalf.filled")
            }
        }
    }

    private var diagnosticSection: some View {
        Section("服务器诊断") {
            LabeledContent("当前连接", value: model.coreStatusText)
            Button {
                Task { await runSelfCheck() }
            } label: {
                HStack {
                    Label(checkingServer ? "正在检测…" : "运行服务器自检", systemImage: "waveform.path.ecg")
                    Spacer()
                    if checkingServer { ProgressView() }
                }
            }
            .disabled(checkingServer)
            if !selfCheckMessage.isEmpty {
                Text(selfCheckMessage)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func runSelfCheck() async {
        guard let client = model.apiClient else {
            selfCheckMessage = "❌ 未配置服务器地址"
            return
        }
        checkingServer = true
        defer { checkingServer = false }
        let start = Date()
        do {
            let status = try await client.health()
            let ms = Int(Date().timeIntervalSince(start) * 1000)
            let modules = (try? await client.listLibraryModules()) ?? []
            selfCheckMessage = "✅ \(status)（\(ms)ms）· 已载入 \(modules.count) 个资料库模块 · \(client.configuration.baseURL.absoluteString)"
        } catch {
            selfCheckMessage = "❌ \(error.localizedDescription)"
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
            Text("服务端 115 直连加速")
        } footer: {
            Text("配置后服务端会通过 AList 解析 115 直链，避开跨境中转带宽。")
        }
    }

    private var cacheSection: some View {
        Section("服务端缓存") {
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
            LabeledContent("版本", value: (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "2.2.0")
            LabeledContent("运行模式", value: "纯服务器模式")
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
        cache = try? await client.cacheStats()
        if let value = try? await client.alistAccelConfig() {
            accel = value
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
