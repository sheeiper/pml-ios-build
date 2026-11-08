import SwiftUI

/// CD2（索引） + AList（加速）双配置，以及四个目录槽位。
struct MediaSourcesView: View {
    @EnvironmentObject private var model: AppModel

    @State private var provider = ProviderConfig(providerType: "cd2")
    @State private var accel = AlistAccelConfig(
        enabled: true,
        apiUrl: "",
        username: "",
        password: nil,
        cd2Prefix: "",
        alistPrefix: ""
    )
    @State private var directories = DirectoryConfigPayload()
    @State private var status: LibraryStatus?
    @State private var busy = false
    @State private var message = ""
    @State private var errorText = ""
    @State private var mapping: AlistMappingCheck?

    @State private var pickerProvider: DirectoryProvider?
    @State private var pickerTitle = ""
    @State private var pickerPath = "/"
    @State private var pickerTarget: DirectorySlot?

    enum DirectoryProvider: String, Identifiable {
        case cd2 = "CD2"
        case alist = "AList"
        var id: String { rawValue }
    }

    enum DirectorySlot: String, Identifiable {
        case recent, pending, vault, comics, cd2Prefix, alistPrefix
        var id: String { rawValue }

        var title: String {
            switch self {
            case .recent: return "最近添加"
            case .pending: return "待整理"
            case .vault: return "已整理"
            case .comics: return "漫画"
            case .cd2Prefix: return "CD2 目录前缀"
            case .alistPrefix: return "AList 目录前缀"
            }
        }
    }

    var body: some View {
        Form {
            cd2Section
            directoriesSection
            alistSection
            serverModeSection

            if !message.isEmpty {
                Section { Text(message).font(.footnote).foregroundStyle(.secondary) }
            }
            if !errorText.isEmpty {
                Section { Text(errorText).font(.footnote).foregroundStyle(.red) }
            }
        }
        .navigationTitle("媒体源")
        .navigationBarTitleDisplayMode(.inline)
        .task { await load() }
        .sheet(item: $pickerProvider) { provider in
            NavigationStack {
                DirectoryBrowserView(
                    provider: provider,
                    title: pickerTitle,
                    startPath: pickerPath
                ) { selected in
                    applyPicker(selected)
                }
            }
        }
    }

    // MARK: - CD2

    private var cd2Section: some View {
        Section {
            LabeledContent("状态", value: status?.connected == true ? "已连接" : "未连接")
            TextField("CD2 WebDAV 地址（http://IP:19798/dav）", text: binding(\.cd2Endpoint))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
            TextField("账号", text: binding(\.cd2User))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            SecureField("密码", text: binding(\.cd2Password))

            Button(busy ? "连接中…" : "测试连接并保存") { Task { await saveCD2() } }
                .disabled(busy)
        } header: {
            Label("CD2（文件索引）", systemImage: "externaldrive.connected.to.line.below")
        } footer: {
            Text("CD2 负责目录索引与 WebDAV 读写；保存成功后才能选择下面四个目录。")
        }
    }

    // MARK: - 四个目录

    private var directoriesSection: some View {
        Section {
            directoryRow(.recent, path: directories.recentPath)
            directoryRow(.pending, path: directories.pendingPath)
            directoryRow(.vault, path: directories.vaultPath)
            directoryRow(.comics, path: directories.comicsPath)

            Button(busy ? "保存中…" : "保存目录配置") { Task { await saveDirectories() } }
                .disabled(busy || !(status?.connected ?? false))
        } header: {
            Text("四个目录")
        } footer: {
            Text("最近添加/待整理不参与扫描；已整理与漫画会生成扫描根目录。")
        }
    }

    private func directoryRow(_ slot: DirectorySlot, path: String?) -> some View {
        Button {
            pickerTarget = slot
            pickerProvider = .cd2
            pickerTitle = "选择\(slot.title)"
            pickerPath = path?.isEmpty == false ? path! : "/"
        } label: {
            HStack {
                Text(slot.title)
                Spacer()
                Text(path?.isEmpty == false ? path! : "未选择")
                    .foregroundStyle(path?.isEmpty == false ? Color.secondary : Color.red)
                    .lineLimit(1)
                    .truncationMode(.head)
                Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
            }
        }
        .buttonStyle(.plain)
    }

    // MARK: - AList

    private var alistSection: some View {
        Section {
            TextField("AList 地址（http://IP:5244）", text: bindingAccel(\.apiUrl))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
            TextField("账号", text: bindingAccel(\.username))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            SecureField("密码", text: accelPasswordBinding)

            Button {
                pickerTarget = .cd2Prefix
                pickerProvider = .cd2
                pickerTitle = "选择 CD2 前缀目录"
                pickerPath = accel.cd2Prefix.isEmpty ? "/" : accel.cd2Prefix
            } label: {
                HStack {
                    Text("CD2 目录前缀")
                    Spacer()
                    Text(accel.cd2Prefix.isEmpty ? "未选择" : accel.cd2Prefix)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .buttonStyle(.plain)

            Button {
                pickerTarget = .alistPrefix
                pickerProvider = .alist
                pickerTitle = "选择 AList 前缀目录"
                pickerPath = accel.alistPrefix.isEmpty ? "/" : accel.alistPrefix
            } label: {
                HStack {
                    Text("AList 目录前缀")
                    Spacer()
                    Text(accel.alistPrefix.isEmpty ? "未选择" : accel.alistPrefix)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .buttonStyle(.plain)

            Button("保存加速配置") { Task { await saveAList() } }
                .disabled(busy)
            Button("检查映射") { Task { await checkMapping() } }
                .disabled(busy || accel.cd2Prefix.isEmpty || accel.alistPrefix.isEmpty)

            if let mapping {
                Text(mapping.message)
                    .font(.caption)
                    .foregroundStyle(mapping.ok ? .green : .orange)
                if !mapping.missingInAlist.isEmpty {
                    Text("CD2 有、AList 无：\(mapping.missingInAlist.joined(separator: "、"))")
                        .font(.caption2).foregroundStyle(.secondary)
                }
                if !mapping.missingInCd2.isEmpty {
                    Text("AList 有、CD2 无：\(mapping.missingInCd2.joined(separator: "、"))")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
        } header: {
            Label("AList（直连加速）", systemImage: "bolt.horizontal.circle")
        } footer: {
            Text("全局前缀替换：CD2 路径前缀 ↔ AList 路径前缀。保存后视频会优先走 AList 直链。")
        }
    }

    private var serverModeSection: some View {
        Section {
            LabeledContent("服务器模式", value: "预留")
                .foregroundStyle(.secondary)
        } footer: {
            Text("后续支持把数据库与索引部署在服务器（Docker / Web），App 作为客户端连接。当前版本暂不启用。")
        }
    }

    // MARK: - 绑定

    private func binding(_ keyPath: WritableKeyPath<ProviderConfig, String?>) -> Binding<String> {
        Binding(
            get: { provider[keyPath: keyPath] ?? "" },
            set: { provider[keyPath: keyPath] = $0.isEmpty ? nil : $0 }
        )
    }

    private func bindingAccel(_ keyPath: WritableKeyPath<AlistAccelConfig, String>) -> Binding<String> {
        Binding(
            get: { accel[keyPath: keyPath] },
            set: { accel[keyPath: keyPath] = $0 }
        )
    }

    private var accelPasswordBinding: Binding<String> {
        Binding(
            get: { accel.password ?? "" },
            set: { accel.password = $0.isEmpty ? nil : $0 }
        )
    }

    // MARK: - 操作

    private func load() async {
        guard let client = model.apiClient else { return }
        status = try? await client.libraryStatus()
        if let existing = try? await client.providerConfig() {
            provider = existing
        }
        if let existing = try? await client.alistAccelConfig() {
            accel = existing
            accel.enabled = true
        }
        if let config = try? await client.directoryConfig() {
            directories = config
        }
    }

    private func saveCD2() async {
        guard let client = model.apiClient else { return }
        busy = true
        errorText = ""
        message = ""
        do {
            provider.providerType = "cd2"
            try await client.saveProviderConfig(provider)
            let restored = try await client.restoreConnection()
            status = restored
            message = restored.connected ? "CD2 连接成功" : "已保存，但连接失败：\(restored.message)"
        } catch {
            errorText = error.localizedDescription
        }
        busy = false
    }

    private func saveDirectories() async {
        guard let client = model.apiClient else { return }
        busy = true
        errorText = ""
        do {
            directoryTargetApplyPending()
            let saved = try await client.saveDirectoryConfig(directories)
            directories = saved
            message = "目录配置已保存"
        } catch {
            errorText = error.localizedDescription
        }
        busy = false
    }

    private func saveAList() async {
        guard let client = model.apiClient else { return }
        busy = true
        errorText = ""
        do {
            accel.enabled = true
            let result = try await client.testAlistAccel(accel)
            if result.ok {
                try await client.saveAlistAccelConfig(accel)
                message = "AList 已保存：\(result.message)"
            } else {
                errorText = "AList 连接失败：\(result.message)"
            }
        } catch {
            errorText = error.localizedDescription
        }
        busy = false
    }

    private func checkMapping() async {
        guard let client = model.apiClient else { return }
        busy = true
        mapping = nil
        do {
            mapping = try await client.validateAlistMapping(cd2Path: accel.cd2Prefix, alistPath: accel.alistPrefix)
        } catch {
            errorText = error.localizedDescription
        }
        busy = false
    }

    private func applyPicker(_ path: String) {
        let normalized = path.isEmpty ? "/" : path
        switch pickerTarget {
        case .recent: directories.recentPath = normalized
        case .pending: directories.pendingPath = normalized
        case .vault: directories.vaultPath = normalized
        case .comics: directories.comicsPath = normalized
        case .cd2Prefix: accel.cd2Prefix = normalized
        case .alistPrefix: accel.alistPrefix = normalized
        case nil: break
        }
        pickerTarget = nil
        pickerProvider = nil
    }

    private func directoryTargetApplyPending() {}
}

/// 目录选择器：面包屑 + 上级 + 选择当前目录。
struct DirectoryBrowserView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let provider: MediaSourcesView.DirectoryProvider
    let title: String
    @State private var path: String
    @State private var directories: [String] = []
    @State private var loading = false
    @State private var errorText = ""

    let onSelect: (String) -> Void

    init(
        provider: MediaSourcesView.DirectoryProvider,
        title: String,
        startPath: String,
        onSelect: @escaping (String) -> Void
    ) {
        self.provider = provider
        self.title = title
        self.onSelect = onSelect
        _path = State(initialValue: startPath.isEmpty ? "/" : startPath)
    }

    var body: some View {
        List {
            Section {
                Text(path).font(.caption).foregroundStyle(.secondary)
                Button("选择当前目录") {
                    onSelect(path)
                    dismiss()
                }
                .disabled(loading)
            }

            Section("子目录") {
                if loading {
                    ProgressView()
                } else if directories.isEmpty {
                    Text("没有子目录").foregroundStyle(.secondary)
                } else {
                    ForEach(directories, id: \.self) { name in
                        Button(name) {
                            path = join(path, name)
                            Task { await load() }
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            if !errorText.isEmpty {
                Section { Text(errorText).font(.footnote).foregroundStyle(.red) }
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button("上级") { goUp() }
                    .disabled(path == "/" || loading)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button("取消") { dismiss() }
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            let listing = provider == .cd2
                ? try await client.listSourceDirectories(path: path)
                : try await client.listAlistDirectories(path: path)
            directories = listing.directories
        } catch {
            errorText = error.localizedDescription
            directories = []
        }
        loading = false
    }

    private func goUp() {
        guard path != "/" else { return }
        let trimmed = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        var parts = trimmed.split(separator: "/").map(String.init)
        if !parts.isEmpty { parts.removeLast() }
        path = "/" + parts.joined(separator: "/")
        Task { await load() }
    }

    private func join(_ base: String, _ name: String) -> String {
        let prefix = base == "/" ? "" : base
        return "\(prefix)/\(name)"
    }
}
