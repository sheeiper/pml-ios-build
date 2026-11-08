import SwiftUI
import UIKit

/// 带内存缓存的远程图片视图：滚动离开再回来时不会重新下载，失败可重试。
struct CachedImageView: View {
    let url: URL?
    var contentMode: ContentMode = .fit
    var minHeight: CGFloat = 240
    var maxPixel: CGFloat? = nil

    @State private var image: UIImage?
    @State private var failed = false
    @State private var retry = 0

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
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
        .task(id: "\(url?.absoluteString ?? "")-\(retry)") {
            guard let url else { return }
            if let loaded = await ImageCache.shared.cachedOrLoad(url, maxPixel: maxPixel) {
                image = loaded
            } else {
                failed = true
            }
        }
    }
}
