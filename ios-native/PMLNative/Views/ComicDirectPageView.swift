import SwiftUI
import UIKit

/// ZIP 漫画单页：只按 AList `/d` Range 直连并本地解压，不回退服务器。
struct ComicDirectPageView: View {
    let assetId: String
    let page: ComicArchivePageInfo

    @EnvironmentObject private var model: AppModel
    @State private var image: UIImage?
    @State private var failed = false
    @State private var sourceLabel: String?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: .infinity)
            } else if failed {
                ContentUnavailableView("这一页无法直连", systemImage: "wifi.exclamationmark", description: Text("不会改走服务器。可返回后重试。"))
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
