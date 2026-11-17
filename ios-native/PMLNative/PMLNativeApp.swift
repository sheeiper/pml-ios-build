import Foundation
import SwiftUI

@main
struct PMLNativeApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model: AppModel

    init() {
        // 纯 Rust 内核：App 自己提供沙盒目录，内核在 127.0.0.1 上起 API。
        let manager = FileManager.default
        let dataDir = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? manager.temporaryDirectory
        let cacheDir = manager.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? manager.temporaryDirectory
        let port = dataDir.path.withCString { dataPointer in
            cacheDir.path.withCString { cachePointer in
                pml_core_start(dataPointer, cachePointer)
            }
        }
        var errorText: String?
        if port <= 0 {
            if let raw = pml_core_last_error() {
                errorText = String(cString: raw)
            } else {
                errorText = "错误代码 \(port)"
            }
        }
        _model = StateObject(wrappedValue: AppModel(corePort: port, coreError: errorText))
    }

    @Environment(\.scenePhase) private var scenePhase
    @AppStorage("pml.privacy.shield") private var privacyShieldEnabled = true

    var body: some Scene {
        WindowGroup {
            ZStack {
                RootTabView()
                    .environmentObject(model)

                if privacyShieldEnabled && scenePhase != .active {
                    PrivacyShieldOverlayView()
                        .transition(.opacity)
                        .zIndex(99999)
                }
            }
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
