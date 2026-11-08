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

    var body: some Scene {
        WindowGroup {
            RootTabView()
                .environmentObject(model)
        }
    }
}
