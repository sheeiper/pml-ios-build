import SwiftUI

struct RemoteSearchView: View {
    let root: LibraryRoot
    let parentPath: String

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var keyword = ""
    @State private var results: [RemoteBrowserEntry] = []
    @State private var cursor: String?
    @State private var searching = false
    @State private var searched = false
    @State private var errorText = ""
    @State private var preview: PreviewItem?
    @State private var imageContext: ImageViewerContext?

    var body: some View {
        NavigationStack {
            List {
                ForEach(results) { entry in
                    Button {
                        Task { await open(entry) }
                    } label: {
                        row(entry)
                    }
                    .buttonStyle(.plain)
                }

                if let cursor {
                    Button(searching ? "加载中…" : "加载更多") {
                        Task { await search(reset: false, cursor: cursor) }
                    }
                    .disabled(searching)
                }

                if results.isEmpty && searched && !searching && errorText.isEmpty {
                    Text("没有匹配结果").foregroundStyle(.secondary)
                }

                if !errorText.isEmpty {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                }
            }
            .navigationTitle("搜索资料库")
            .searchable(text: $keyword, prompt: "输入文件名关键字")
            .onSubmit(of: .search) {
                Task { await search(reset: true) }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("关闭") { dismiss() }
                }
            }
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
    }

    private func row(_ entry: RemoteBrowserEntry) -> some View {
        HStack(spacing: 12) {
            Image(systemName: entry.isDirectory ? "folder.fill" : (entry.isVideo ? "play.rectangle.fill" : "photo.fill"))
                .foregroundStyle(.tint)
                .frame(width: 26)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.name).lineLimit(1)
                Text(entry.path)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 2)
    }

    private func search(reset: Bool, cursor cursorValue: String? = nil) async {
        guard let client = model.apiClient, !keyword.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        searching = true
        errorText = ""
        if reset {
            results = []
            cursor = nil
            searched = true
        }
        do {
            let page = try await client.searchRemoteFiles(
                rootId: root.id,
                parentPath: parentPath,
                keywords: keyword,
                cursor: cursorValue
            )
            if reset {
                results = page.items
            } else {
                results.append(contentsOf: page.items)
            }
            cursor = page.nextCursor
        } catch {
            errorText = error.localizedDescription
        }
        searching = false
    }

    private func open(_ entry: RemoteBrowserEntry) async {
        guard let assetId = entry.assetId else { return }
        if entry.isImage {
            let single = RemoteBrowserEntry(
                path: entry.path,
                name: entry.name,
                isDirectory: false,
                sizeBytes: entry.sizeBytes,
                modifiedAt: entry.modifiedAt,
                etag: entry.etag,
                assetId: assetId,
                mediaType: entry.mediaType,
                thumbnailSource: entry.thumbnailSource,
                coverAssetId: entry.coverAssetId
            )
            imageContext = ImageViewerContext(entries: [single], startIndex: 0)
            return
        }
        guard let source = await model.playbackSource(assetId: assetId, remotePath: entry.path) else { return }
        preview = PreviewItem(
            url: source.url,
            isVideo: entry.isVideo,
            assetId: assetId,
            title: entry.name,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: entry.path
        )
    }
}
