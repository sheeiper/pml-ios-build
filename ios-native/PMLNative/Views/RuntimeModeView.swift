import SwiftUI

/// 本地 / 服务器运行模式，以及服务器模式下的 AList 直连配置。
struct RuntimeModeView: View {
    @EnvironmentObject private var model: AppModel

    @State private var draft: RuntimeSettings
    @State private var busy = false
    @State private var message = ""
    @State private var errorText = ""

    init(settings: RuntimeSettings) {
        _draft = State(initialValue: settings)
    }

    var body: some View {
        Form {
            Section {
                Picker("运行模式", selection: $draft.mode) {
                    ForEach(AppRunMode.allCases, id: \.self) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
            } header: {
                Text("运行模式")
            } footer: {
                Text("服务器模式下，浏览/索引/进度/字幕来自服务器；本地模式使用 App 内置数据库。切换后全局生效。")
            }

            Section {
                TextField("服务器地址", text: $draft.serverURL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                TextField("账号", text: $draft.serverUser)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                SecureField("密码", text: $draft.serverPassword)
                Button(busy ? "检测中…" : "测试服务器连接") {
                    Task { await testServer() }
                }
                .disabled(busy)
            } header: {
                Text("服务器")
            } footer: {
                Text("服务器负责 CD2 索引与数据库；服务器模式下 App 不需要配置 CD2。")
            }

            Section {
                TextField("AList 地址（http://IP:5244）", text: $draft.alistURL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                TextField("CD2 路径前缀", text: $draft.cd2Prefix)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                TextField("AList 路径前缀", text: $draft.alistPrefix)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button(busy ? "检测中…" : "测试 AList") {
                    Task { await testAList() }
                }
                .disabled(busy)
            } header: {
                Text("AList 直连")
            } footer: {
                Text("播放时把 CD2 路径按前缀映射到 AList，再走 /d 直链；AList 不可用时回退服务器中转。")
            }

            Section {
                Button(busy ? "保存中…" : "保存并应用") {
                    model.updateSettings(draft)
                    message = "已保存。返回并重新进入页面后全局生效。"
                }
                .disabled(busy)
            }

            if !message.isEmpty {
                Section { Text(message).font(.footnote).foregroundStyle(.secondary) }
            }
            if !errorText.isEmpty {
                Section { Text(errorText).font(.footnote).foregroundStyle(.red) }
            }
        }
        .navigationTitle("运行模式与服务器")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func testServer() async {
        guard let url = draft.normalizedServerURL else {
            errorText = "服务器地址无效"
            return
        }
        busy = true
        errorText = ""
        message = ""
        let client = PMLAPIClient(
            configuration: .init(baseURL: url, username: draft.serverUser, password: draft.serverPassword)
        )
        do {
            let status = try await client.health()
            message = "服务器连接成功：\(status) · \(url.absoluteString)"
        } catch {
            errorText = "服务器连接失败：\(error.localizedDescription)"
        }
        busy = false
    }

    private func testAList() async {
        guard let url = draft.normalizedAlistURL else {
            errorText = "AList 地址无效"
            return
        }
        busy = true
        errorText = ""
        message = ""
        var request = URLRequest(url: url.appendingPathComponent("api/public/settings"))
        request.timeoutInterval = 8
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                errorText = "AList 返回异常状态码"
                busy = false
                return
            }
            message = "AList 连接成功 · \(url.absoluteString)"
        } catch {
            errorText = "AList 连接失败：\(error.localizedDescription)"
        }
        busy = false
    }
}