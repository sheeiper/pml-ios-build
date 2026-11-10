import SwiftUI

struct FavoritesView: View {
    @EnvironmentObject private var model: AppModel

    @State private var assets: [LibraryAsset] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var preview: PreviewItem?
    @State private var imageContext: ImageViewerContext?

    @ObservedObject private var thumbs = ThumbnailService.shared

    private let columns = [GridItem(.adaptive(minimum: 104), spacing: 10)]

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 10) {
                ForEach(assets) { asset in
                    cell(asset)
                }
            }
            .padding(12)

            if assets.isEmpty && !loading && errorText.isEmpty {
                Text("还没有收藏")
                    .foregroundStyle(.secondary)
                    .padding(40)
            }

            if !errorText.isEmpty {
                Text(errorText)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .padding()
            }
        }
        .navigationTitle("收藏")
        .task { await load() }
        .refreshable { await load() }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(
                url: item.url,
                title: item.title,
                assetId: item.assetId,
                startAt: item.startAt, fallbackURL: item.fallbackURL, sourceLabel: item.sourceLabel, remotePath: item.remotePath
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $imageContext) { context in
            ImageViewer(context: context)
                .environmentObject(model)
        }
    }

    private func cell(_ asset: LibraryAsset) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(.quaternary.opacity(0.3))
                if let image = thumbs.images[asset.id] {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: asset.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                        .foregroundStyle(.tint)
                }
            }
            .frame(height: 104)
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            Text(asset.fileName)
                .font(.caption)
                .lineLimit(2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task {
            await thumbs.image(assetId: asset.id, client: model.apiClient)
        }
        .onTapGesture {
            Task { await open(asset) }
        }
        .contextMenu {
            Button(role: .destructive) {
                Task { await unfavorite(asset) }
            } label: {
                Label("取消收藏", systemImage: "heart.slash")
            }
        }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            assets = try await client.favoriteAssets(limit: 200)
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }

    private func open(_ asset: LibraryAsset) async {
        guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else { return }
        if asset.mediaType == "video" {
            preview = PreviewItem(
                url: source.url,
                isVideo: true,
                assetId: asset.id,
                title: asset.fileName,
                fallbackURL: source.fallbackURL,
                sourceLabel: source.label,
                remotePath: asset.remotePath
            )
        } else {
            let entry = RemoteBrowserEntry(
                path: asset.remotePath,
                name: asset.fileName,
                isDirectory: false,
                sizeBytes: asset.sizeBytes,
                modifiedAt: nil,
                etag: nil,
                assetId: asset.id,
                mediaType: asset.mediaType,
                thumbnailSource: nil,
                coverAssetId: asset.thumbnailAssetId
            )
            imageContext = ImageViewerContext(entries: [entry], startIndex: 0)
        }
    }

    private func unfavorite(_ asset: LibraryAsset) async {
        guard let client = model.apiClient else { return }
        _ = try? await client.toggleFavorite(assetId: asset.id)
        assets.removeAll { $0.id == asset.id }
    }
}
