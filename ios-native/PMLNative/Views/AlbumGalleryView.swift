import SwiftUI

public struct AlbumGalleryView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var appModel: AppModel

    public let dirPath: String

    @State private var loading = false
    @State private var errorMessage: String?
    @State private var albumInfo: AlbumInfo?
    @State private var activeIndex: Int?

    private let columns = [
        GridItem(.adaptive(minimum: 105, maximum: 130), spacing: 8)
    ]

    public init(dirPath: String) {
        self.dirPath = dirPath
    }

    public var body: some View {
        NavigationStack {
            contentView
                .navigationTitle(albumInfo?.title ?? "相册画册图集")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("完成") { dismiss() }
                    }
                }
                .sheet(isPresented: Binding(
                    get: { activeIndex != nil },
                    set: { if !$0 { activeIndex = nil } }
                )) {
                    if let album = albumInfo, let currentIdx = activeIndex {
                        fullScreenViewer(album: album, currentIdx: currentIdx)
                    }
                }
                .task {
                    await loadAlbum()
                }
        }
    }

    @ViewBuilder
    private var contentView: some View {
        if loading {
            loadingView
        } else if let error = errorMessage {
            errorView(error)
        } else if let album = albumInfo {
            albumScrollView(album)
        } else {
            EmptyView()
        }
    }

    private var loadingView: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text("正在整理画册集并加载图片...")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func errorView(_ error: String) -> some View {
        VStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 36))
                .foregroundStyle(.orange)
            Text(error)
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Button("重试") {
                Task { await loadAlbum() }
            }
            .buttonStyle(.bordered)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func albumScrollView(_ album: AlbumInfo) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 8) {
                    Text("\(album.imageCount) 张照片")
                        .font(.caption.bold())
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Color.indigo.opacity(0.18))
                        .foregroundStyle(Color.indigo)
                        .clipShape(Capsule())

                    Text(formatBytes(album.totalSizeBytes))
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Color.secondary.opacity(0.12))
                        .foregroundStyle(.secondary)
                        .clipShape(Capsule())
                }
                .padding(.horizontal)
                .padding(.top, 4)

                LazyVGrid(columns: columns, spacing: 8) {
                    ForEach(Array(album.items.enumerated()), id: \.element.assetId) { idx, item in
                        Button {
                            activeIndex = idx
                        } label: {
                            thumbnailCell(item: item, index: idx)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal)
            }
        }
    }

    private func thumbnailCell(item: AlbumItem, index: Int) -> some View {
        ZStack(alignment: .bottomTrailing) {
            if let url = appModel.thumbnailURL(for: item.assetId) {
                AsyncImage(url: url) { phase in
                    if let img = phase.image {
                        img.resizable().scaledToFill()
                    } else {
                        Rectangle().fill(Color.secondary.opacity(0.15))
                    }
                }
                .frame(height: 140)
                .clipped()
            } else {
                Rectangle().fill(Color.secondary.opacity(0.15))
                    .frame(height: 140)
            }

            Text("\(index + 1)")
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(Color.black.opacity(0.65))
                .clipShape(RoundedRectangle(cornerRadius: 3))
                .padding(4)
        }
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private func fullScreenViewer(album: AlbumInfo, currentIdx: Int) -> some View {
        NavigationStack {
            TabView(selection: Binding(
                get: { currentIdx },
                set: { activeIndex = $0 }
            )) {
                ForEach(Array(album.items.enumerated()), id: \.offset) { idx, item in
                    ZStack {
                        Color.black.ignoresSafeArea()
                        if let url = appModel.mediaSourceURL(for: item.assetId) {
                            AsyncImage(url: url) { phase in
                                if let img = phase.image {
                                    img.resizable().scaledToFit()
                                } else {
                                    ProgressView().tint(.white)
                                }
                            }
                        }
                    }
                    .tag(idx)
                }
            }
            .tabViewStyle(.page(indexDisplayMode: .automatic))
            .navigationTitle("\(currentIdx + 1) / \(album.items.count)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("关闭") { activeIndex = nil }
                        .foregroundStyle(.white)
                }
            }
        }
    }

    private func loadAlbum() async {
        guard let client = appModel.apiClient else {
            errorMessage = "未连接到数据源"
            return
        }
        loading = true
        errorMessage = nil
        do {
            albumInfo = try await client.getAlbumDetails(dirPath: dirPath)
        } catch {
            errorMessage = error.localizedDescription
        }
        loading = false
    }

    private func formatBytes(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}
