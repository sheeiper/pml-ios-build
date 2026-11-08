import SwiftUI

/// 媒体源配置：CloudDrive2 WebDAV 或 AList，保存后立即尝试连接。
struct MediaSourceSetupView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var config = ProviderConfig(providerType: "cd2")
    @State private var loading = false
    @State private var message = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("类型") {
                    Picker("媒体源类型", selection: $config.providerType) {
                        Text("CloudDrive2 (WebDAV)").tag("cd2")
                        Text("AList").tag("alist")
                    }
                    .pickerStyle(.segmented)
                }

                if config.providerType == "cd2" {
                    Section {
                        TextField("地址（http://192.168.1.10:19798/dav/）", text: binding(\.cd2Endpoint))
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        TextField("账号（可留空）", text: binding(\.cd2User))
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        SecureField("密码（可留空）", text: binding(\.cd2Password))
                    } header: {
                        Text("CloudDrive2")
                    } footer: {
                        Text("CloudDrive2 开启 WebDAV 账号验证后必须填写账号密码，否则连接会返回 401。公开地址（如 103.119.1.41）通常已开启验证。")
                    }
                } else {
                    Section("AList") {
                        TextField("地址（http://192.168.1.10:5244）", text: binding(\.alistEndpoint))
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        SecureField("只读 Token", text: binding(\.alistToken))
                    }
                }

                Section {
                    Button(loading ? "正在保存…" : "保存并连接") {
                        Task { await save() }
                    }
                    .disabled(loading)
                }

                if !message.isEmpty {
                    Section {
                        Text(message)
                            .font(.footnote)
                            .foregroundStyle(message.hasPrefix("已连接") ? Color.green : Color.secondary)
                    }
                }
            }
            .navigationTitle("媒体源")
            .task { await load() }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("关闭") { dismiss() }
                }
            }
        }
    }

    private func binding(_ keyPath: WritableKeyPath<ProviderConfig, String?>) -> Binding<String> {
        Binding(
            get: { config[keyPath: keyPath] ?? "" },
            set: { config[keyPath: keyPath] = $0.isEmpty ? nil : $0 }
        )
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        if let existing = try? await client.providerConfig() {
            config = existing
        }
    }

    private func save() async {
        guard let client = model.apiClient else {
            message = "数据源未就绪"
            return
        }
        // 支持 http://user:pass@host 形式：把 URL 里的凭据拆到独立字段。
        if config.providerType == "cd2",
           let endpoint = config.cd2Endpoint?.trimmingCharacters(in: .whitespacesAndNewlines),
           var components = URLComponents(string: endpoint),
           let user = components.user, !user.isEmpty {
            config.cd2User = user
            if let password = components.password, !password.isEmpty {
                config.cd2Password = password
            }
            components.user = nil
            components.password = nil
            config.cd2Endpoint = components.string
        }
        loading = true
        do {
            try await client.saveProviderConfig(config)
            let status = try await client.restoreConnection()
            message = status.connected ? "已连接：\(status.message)" : "连接失败：\(status.message)"
            if status.connected { dismiss() }
        } catch {
            message = error.localizedDescription
        }
        loading = false
    }
}

/// 添加资料库根目录（媒体 / 漫画）。
struct AddRootView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var onAdded: () -> Void

    @State private var kind = "media"
    @State private var path = ""
    @State private var loading = false
    @State private var message = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("类型") {
                    Picker("根目录类型", selection: $kind) {
                        Text("综合媒体").tag("media")
                        Text("漫画").tag("comics")
                    }
                    .pickerStyle(.segmented)
                }

                Section {
                    Text("综合媒体只索引图片和视频；漫画类型索引图片目录与 CBZ/ZIP 压缩包。选错类型会导致目录里看起来是空的。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                Section("远端路径") {
                    TextField("/115open/media", text: $path)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }

                Section {
                    Button(loading ? "添加中…" : "添加") {
                        Task { await add() }
                    }
                    .disabled(loading || path.trimmingCharacters(in: .whitespaces).isEmpty)
                }

                if !message.isEmpty {
                    Section {
                        Text(message)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("添加根目录")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { dismiss() }
                }
            }
        }
    }

    private func add() async {
        guard let client = model.apiClient else { return }
        loading = true
        do {
            try await client.addLibraryRoot(kind: kind, remotePath: path)
            onAdded()
            dismiss()
        } catch {
            message = error.localizedDescription
        }
        loading = false
    }
}
