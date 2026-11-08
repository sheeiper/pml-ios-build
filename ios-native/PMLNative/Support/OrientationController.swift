import AVKit
import SwiftUI
import UIKit

/// 全局方向控制：播放器允许全向，其他页面锁定竖屏。
final class OrientationController {
    static let shared = OrientationController()

    var mask: UIInterfaceOrientationMask = .portrait

    func lock(_ mask: UIInterfaceOrientationMask) {
        self.mask = mask
            request(mask)
    }

    func request(_ mask: UIInterfaceOrientationMask) {
        guard let scene = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .first else { return }
        scene.requestGeometryUpdate(.iOS(interfaceOrientations: mask)) { _ in }
        UIViewController.attemptRotationToDeviceOrientation()
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        supportedInterfaceOrientationsFor window: UIWindow?
    ) -> UIInterfaceOrientationMask {
        OrientationController.shared.mask
    }
}

struct SystemPlayerScreen: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        let player = AVPlayer(url: url)
        controller.player = player
        controller.modalPresentationStyle = .fullScreen
        player.play()
        return controller
    }

    func updateUIViewController(_ uiViewController: AVPlayerViewController, context: Context) {}
}
