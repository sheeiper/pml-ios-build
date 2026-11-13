import SwiftUI
import UIKit

/// 带内存缓存的远程图片视图：同步命中内存缓存零闪烁，滚动离开再回来时不会重新下载，失败可重试。
struct CachedImageView: View {
    let url: URL?
    var contentMode: ContentMode = .fit
    var minHeight: CGFloat = 240
    var maxPixel: CGFloat? = 1800

    @State private var image: UIImage?
    @State private var loadedKey: String?
    @State private var failed = false
    @State private var retry = 0

    private var currentKey: String {
        "\(url?.absoluteString ?? "")|\(maxPixel.map { Int($0.rounded()) } ?? 0)|\(retry)"
    }

    private var displayImage: UIImage? {
        if loadedKey == currentKey, let image {
            return image
        }
        guard let url else { return nil }
        return ImageCache.shared.cached(url, maxPixel: maxPixel)
    }

    var body: some View {
        Group {
            if let current = displayImage {
                Image(uiImage: current)
                    .resizable()
                    .aspectRatio(contentMode: contentMode)
            } else if failed {
                VStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle")
                    Text("图片加载失败").font(.caption)
                    Button("重试") {
                        failed = false
                        image = nil
                        retry += 1
                    }
                    .font(.caption)
                }
                .frame(maxWidth: .infinity, minHeight: minHeight)
                .foregroundStyle(.secondary)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, minHeight: minHeight)
            }
        }
        .task(id: currentKey) {
            guard let url else { return }
            let key = currentKey
            if let hit = ImageCache.shared.cached(url, maxPixel: maxPixel) {
                image = hit
                loadedKey = key
                failed = false
                return
            }
            if let loaded = await ImageCache.shared.cachedOrLoad(url, maxPixel: maxPixel) {
                guard !Task.isCancelled else { return }
                image = loaded
                loadedKey = key
                failed = false
            } else if !Task.isCancelled {
                failed = true
            }
        }
    }
}
