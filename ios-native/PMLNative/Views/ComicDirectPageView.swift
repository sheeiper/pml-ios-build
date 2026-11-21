import SwiftUI
import UIKit

/// ZIP 漫画单页：只按 AList `/d` Range 直连并本地解压，不回退服务器。
struct ComicDirectPageView: View {
    let assetId: String
    let page: ComicArchivePageInfo

    @EnvironmentObject private var model: AppModel
    @State private var image: UIImage?
    @State private var failed = false

    private var serverPageURL: URL? {
        model.apiClient?.comicPageURL(assetId: assetId, pageIndex: page.index)
    }

    private var displayImage: UIImage? {
        if let image { return image }
        return ComicDirectStore.shared.cachedImage(assetId: assetId, index: page.index)
    }

    var body: some View {
        Group {
            if let current = displayImage {
                Image(uiImage: current)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: .infinity)
            } else if failed, let serverPageURL {
                // Direct AList ZIP range is preferred, but the authenticated
                // comic-page route keeps legacy ZIP/ZIP64/encrypted archives
                // readable without sending credentials to AList/CDN.
                CachedImageView(url: serverPageURL, contentMode: .fit, minHeight: 360, maxPixel: 2400)
            } else if failed {
                ContentUnavailableView("这一页无法打开", systemImage: "wifi.exclamationmark", description: Text("直链和服务器图片接口都不可用，请检查服务连接后重试。"))
                    .frame(maxWidth: .infinity, minHeight: 360)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, minHeight: 360)
            }
        }
        .task(id: page.index) {
            await load()
        }
    }

    private func load() async {
        if let cached = ComicDirectStore.shared.cachedImage(assetId: assetId, index: page.index) {
            image = cached
            failed = false
            return
        }
        if let loaded = await ComicDirectStore.shared.image(assetId: assetId, index: page.index, model: model) {
            image = loaded
            failed = false
        } else {
            failed = true
        }
    }
}
