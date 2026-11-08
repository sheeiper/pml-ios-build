import SwiftUI
import UIKit

/// ZIP 漫画单页：优先按页直连 + 本地解压；失败时自动回退服务器漫画页路由。
struct ComicDirectPageView: View {
    let assetId: String
    let page: ComicArchivePageInfo

    @EnvironmentObject private var model: AppModel
    @State private var image: UIImage?
    @State private var failed = false

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: .infinity)
            } else if failed {
                CachedImageView(
                    url: fallbackURL,
                    contentMode: .fit,
                    minHeight: 360,
                    maxPixel: 1600
                )
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, minHeight: 360)
            }
        }
        .task(id: page.index) {
            await load()
        }
    }

    private var fallbackURL: URL? {
        model.apiClient?.comicPageURL(assetId: assetId, pageIndex: page.index)
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
