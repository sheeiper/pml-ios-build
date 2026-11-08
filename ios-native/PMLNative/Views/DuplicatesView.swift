import SwiftUI

struct DuplicatesView: View {
    @EnvironmentObject private var model: AppModel

    @State private var groups: [DuplicateGroup] = []
    @State private var progress: DuplicateScanProgress?
    @State private var scanning = false
    @State private var loading = false
    @State private var errorText = ""
    @State private var preview: PreviewItem?
    @State private var imageContext: ImageViewerContext?

    @ObservedObject private var thumbs = ThumbnailService.shared

    var body: some View {
        List {
            Section {
                if let progress {
                    ProgressView(value: progress.percent, total: 100)
                    Text(progress.message)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Button(scanning ? "扫描中…" : "开始重复文件扫描") {
                    Task { await scan() }
                }
                .disabled(scanning)
                if scanning {
                    Button("取消扫描", role: .destructive) {
                        Task { await cancel() }
                    }
                }
            }

            ForEach(groups) { group in
                Section {
                    ForEach(group.assets) { asset in
                        Button {
                            Task { await open(asset) }
                        } label: {
                            row(asset)
                        }
                        .buttonStyle(.plain)
                        .task {
                            await thumbs.image(assetId: asset.id, client: model.apiClient)
                        }
                    }
                } header: {
                    Text("\(group.assets.count) 个副本 · 每个 \(pmlFormatBytes(UInt64(max(0, group.sizeBytes))))")
                }
            }

            if groups.isEmpty && !loading {
                Text("没有发现重复文件").foregroundStyle(.secondary)
            }

            if !errorText.isEmpty {
                Text(errorText).font(.footnote).foregroundStyle(.red)
            }
        }
        .navigationTitle("重复文件")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { await loadGroups() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
            }
        }
        .task { await loadGroups() }
        .fullScreenCover(item: $preview) { item in
            GestureVideoPlayerSheet(
                url: item.url,
                title: item.title,
                assetId: item.assetId,
                startAt: item.startAt, fallbackURL: item.fallbackURL, sourceLabel: item.sourceLabel
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $imageContext) { context in
            ImageViewer(context: context)
                .environmentObject(model)
        }
    }

    private func row(_ asset: DuplicateAsset) -> some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(.quaternary.opacity(0.3))
                if let image = thumbs.images[asset.id] {
                    Image(uiImage: image).resizable().scaledToFill()
                } else {
                    Image(systemName: asset.mediaType == "video" ? "play.rectangle.fill" : "photo.fill")
                        .foregroundStyle(.tint)
                }
            }
            .frame(width: 46, height: 46)
            .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))

            VStack(alignment: .leading, spacing: 2) {
                Text(asset.fileName).lineLimit(1)
                Text(asset.parentPath)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 2)
    }

    private func loadGroups() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            groups = try await client.duplicateGroups()
            progress = try? await client.duplicateScanProgress()
            scanning = progress?.running ?? false
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loading = false
    }

    private func scan() async {
        guard let client = model.apiClient, !scanning else { return }
        scanning = true
        errorText = ""
        do {
            progress = try await client.startDuplicateScan()
            for _ in 0..<1200 {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard let latest = try? await client.duplicateScanProgress() else { break }
                progress = latest
                if !latest.running {
                    scanning = false
                    if let error = latest.error { errorText = error }
                    break
                }
            }
            await loadGroups()
        } catch {
            errorText = error.localizedDescription
        }
        scanning = false
    }

    private func cancel() async {
        guard let client = model.apiClient else { return }
        try? await client.cancelDuplicateScan()
        scanning = false
        await loadGroups()
    }

    private func open(_ asset: DuplicateAsset) async {
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
                modifiedAt: asset.modifiedAt,
                etag: nil,
                assetId: asset.id,
                mediaType: asset.mediaType,
                thumbnailSource: nil,
                coverAssetId: nil
            )
            imageContext = ImageViewerContext(entries: [entry], startIndex: 0)
        }
    }
}
