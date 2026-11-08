import SwiftUI

struct HistoryView: View {
    @EnvironmentObject private var model: AppModel

    @State private var items: [PlaybackHistoryItem] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var preview: PreviewItem?
    @StateObject private var thumbs = ThumbnailLoader()

    var body: some View {
        List {
            ForEach(items) { item in
                Button {
                    Task { await open(item) }
                } label: {
                    row(item)
                }
                .buttonStyle(.plain)
                .task {
                    await thumbs.image(assetId: item.asset.thumbnailAssetId ?? item.asset.id, client: model.apiClient)
                }
            }

            if items.isEmpty && !loading && errorText.isEmpty {
                Text("暂无播放记录").foregroundStyle(.secondary)
            }

            if !errorText.isEmpty {
                Text(errorText).font(.footnote).foregroundStyle(.red)
            }
        }
        .navigationTitle("播放记录")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(role: .destructive) {
                    Task { await clearAll() }
                } label: {
                    Image(systemName: "trash")
                }
                .disabled(items.isEmpty)
            }
        }
        .task { await load() }
        .refreshable { await load() }
        .fullScreenCover(item: $preview) { item in
            if item.isVideo {
                GestureVideoPlayerSheet(
                    url: item.url,
                    title: item.title,
                    assetId: item.assetId,
                    startAt: item.startAt, fallbackURL: item.fallbackURL, sourceLabel: item.sourceLabel
                )
                .environmentObject(model)
            } else {
                ImageViewerSheet(url: item.url)
            }
        }
    }

    private func row(_ item: PlaybackHistoryItem) -> some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(.quaternary.opacity(0.35))
                if let image = thumbs.images[item.asset.thumbnailAssetId ?? item.asset.id] {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFill()
                } else {
                    Image(systemName: item.asset.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                        .foregroundStyle(.tint)
                }
            }
            .frame(width: 46, height: 46)
            .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))

            VStack(alignment: .leading, spacing: 3) {
                Text(item.asset.fileName).lineLimit(1)
                HStack(spacing: 8) {
                    Text(pmlFormatTime(item.positionSeconds))
                    if let author = item.asset.authorName, !author.isEmpty {
                        Text(author)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Spacer(minLength: 0)
        }
        .padding(.vertical, 2)
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            items = try await client.playbackHistory(limit: 100)
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }

    private func open(_ item: PlaybackHistoryItem) async {
        guard let source = await model.playbackSource(assetId: item.asset.id, remotePath: item.asset.remotePath) else { return }
        preview = PreviewItem(
            url: source.url,
            isVideo: item.asset.mediaType == "video",
            startAt: Double(item.positionSeconds),
            assetId: item.asset.id,
            title: item.asset.fileName,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: item.asset.remotePath
        )
    }

    private func clearAll() async {
        guard let client = model.apiClient else { return }
        try? await client.clearPlaybackHistory()
        await load()
    }
}
