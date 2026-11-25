import SwiftUI

/// 服务器连接与 AList 直连配置（纯服务器模式）。
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
                Text("浏览、索引、进度与五大资料库均由服务器统一维护；视频严格走直连，不经服务器中转。")
            }

            Section {
                TextField("公网 AList 地址（http://IP:5244）", text: $draft.alistURL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                TextField(
                    "内网 AList 地址（可选，如 http://192.168.1.x:5244）",
                    text: Binding(
                        get: { draft.lanAlistURL ?? "" },
                        set: { draft.lanAlistURL = $0 }
                    )
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
                TextField("CD2 路径前缀", text: $draft.cd2Prefix)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                TextField("AList 路径前缀", text: $draft.alistPrefix)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                HStack {
                    Text("当前直连路由")
                    Spacer()
                    Text(model.isOnLanAlist ? "局域网 AList 直连" : "公网 AList 直连")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(model.isOnLanAlist ? .green : .secondary)
                }
                Button(busy ? "检测中…" : "测试公网 AList") {
                    Task { await testAList(url: draft.normalizedAlistURL, label: "公网 AList") }
                }
                .disabled(busy)
                if draft.normalizedLanAlistURL != nil {
                    Button(busy ? "检测中…" : "测试内网 AList") {
                        Task { await testAList(url: draft.normalizedLanAlistURL, label: "内网 AList") }
                    }
                    .disabled(busy)
                }
            } header: {
                Text("AList 直连（支持内网自动切换）")
            } footer: {
                Text("填写内网 AList 地址后，手机连入同局域网 Wi-Fi 会自动探活并切换局域网直连，切回蜂窝网络自动回退公网。")
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
        .navigationTitle("服务器与直连设置")
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

    private func testAList(url: URL?, label: String) async {
        guard let url else {
            errorText = "\(label)地址无效"
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
                errorText = "\(label)返回异常状态码"
                busy = false
                return
            }
            message = "\(label)连接成功 · \(url.absoluteString)"
        } catch {
            errorText = "\(label)连接失败：\(error.localizedDescription)"
        }
        busy = false
    }
}
