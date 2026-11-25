import Foundation
import SwiftUI

@main
struct PMLNativeApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model = AppModel()

    @Environment(\.scenePhase) private var scenePhase
    @AppStorage("pml.privacy.shield") private var privacyShieldEnabled = true

    var body: some Scene {
        WindowGroup {
            ZStack {
                if model.isConfigured {
                    RootTabView()
                        .environmentObject(model)
                } else {
                    ServerOnboardingView()
                        .environmentObject(model)
                }

                if privacyShieldEnabled && scenePhase != .active {
                    PrivacyShieldOverlayView()
                        .transition(.opacity)
                        .zIndex(99999)
                }
            }
            .alert("无法直链播放", isPresented: Binding(
                get: { model.playbackError != nil },
                set: { if !$0 { model.playbackError = nil } }
            )) {
                Button("知道了", role: .cancel) { model.playbackError = nil }
            } message: {
                Text(model.playbackError ?? DirectPlaybackPolicy.failureMessage)
            }
        }
    }
}

/// 首次启动连接服务器引导页（纯服务器模式）。
private struct ServerOnboardingView: View {
    @EnvironmentObject private var model: AppModel
    @State private var draft: RuntimeSettings = .load()
    @State private var testing = false
    @State private var errorText = ""

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(spacing: 10) {
                        Image(systemName: "server.rack")
                            .font(.system(size: 42, weight: .semibold))
                            .foregroundStyle(.tint)
                        Text("连接私人媒体库服务器")
                            .font(.title3.weight(.bold))
                        Text("填写服务器地址与登录凭据即可同步全部五大资料库与直连播放。")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                }

                Section("服务器凭据") {
                    TextField("服务器地址（http://IP:18080）", text: $draft.serverURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    TextField("账号", text: $draft.serverUser)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("密码", text: $draft.serverPassword)
                }

                Section("AList 直连（选填，用于视频直链加速）") {
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
                }

                if !errorText.isEmpty {
                    Section {
                        Text(errorText)
                            .font(.footnote)
                            .foregroundStyle(.red)
                    }
                }

                Section {
                    Button {
                        Task { await connectAndSave() }
                    } label: {
                        HStack {
                            Spacer()
                            if testing {
                                ProgressView().padding(.trailing, 6)
                            }
                            Text(testing ? "正在验证连接…" : "连接并进入媒体库")
                                .fontWeight(.semibold)
                            Spacer()
                        }
                    }
                    .disabled(testing)
                }
            }
            .navigationTitle("服务器配置")
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private func connectAndSave() async {
        guard let url = draft.normalizedServerURL else {
            errorText = "请填写有效的 http(s) 服务器地址"
            return
        }
        guard !draft.serverPassword.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            errorText = "请填写服务器登录密码"
            return
        }
        testing = true
        errorText = ""
        defer { testing = false }
        let client = PMLAPIClient(
            configuration: .init(
                baseURL: url,
                username: draft.serverUser,
                password: draft.serverPassword
            )
        )
        do {
            _ = try await client.health()
            model.updateSettings(draft)
        } catch {
            errorText = "连接失败：\(error.localizedDescription)"
        }
    }
}

/// 后台多任务防窥遮罩：当 App 切入多任务卡片或锁屏时，拦截系统截图，杜绝敏感内容泄露。
private struct PrivacyShieldOverlayView: View {
    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            Rectangle()
                .fill(.ultraThinMaterial)
                .ignoresSafeArea()
            VStack(spacing: 14) {
                Image(systemName: "lock.shield.fill")
                    .font(.system(size: 52, weight: .medium))
                    .foregroundStyle(.tint)
                Text("私人媒体中心")
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(.white)
                Text("隐私保护已激活")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}
