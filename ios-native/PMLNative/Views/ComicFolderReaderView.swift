import SwiftUI

struct ComicFolderTarget: Identifiable {
    let id = UUID()
    let root: LibraryRoot
    let path: String
    let startPath: String?
}

enum ComicReadingMode: String, CaseIterable, Identifiable {
    case webtoon = "条漫"
    case single = "单页"
    case double = "双页"

    var id: String { rawValue }

    var symbol: String {
        switch self {
        case .webtoon: return "rectangle.portrait"
        case .single: return "rectangle"
        case .double: return "rectangle.split.3x1"
        }
    }
}

/// 图片文件夹漫画：复用统一极简阅读器（缩放、原图直出+渐进增强动画、Metal 画质增强、单/双/条漫/日漫）。
struct ComicFolderReaderView: View {
    let target: ComicFolderTarget

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var pageSources: [ReaderPageSource] = []
    @State private var initialIndex = 0
    @State private var loading = true
    @State private var errorText = ""

    var body: some View {
        Group {
            if loading && pageSources.isEmpty {
                ZStack {
                    Color.black.ignoresSafeArea()
                    ProgressView("加载目录…")
                        .tint(.white)
                        .foregroundStyle(.white)
                }
            } else if !errorText.isEmpty {
                NavigationStack {
                    ContentUnavailableView(
                        "无法打开漫画",
                        systemImage: "exclamationmark.triangle",
                        description: Text(errorText)
                    )
                    .toolbar {
                        ToolbarItem(placement: .topBarTrailing) { Button("关闭") { dismiss() } }
                    }
                }
            } else if pageSources.isEmpty {
                NavigationStack {
                    ContentUnavailableView("没有图片页", systemImage: "photo.on.rectangle")
                        .toolbar {
                            ToolbarItem(placement: .topBarTrailing) { Button("关闭") { dismiss() } }
                        }
                }
            } else {
                ComicGalleryReaderView(
                    kind: .comic,
                    title: title,
                    sources: pageSources,
                    initialPage: initialIndex
                )
            }
        }
        .task { await load() }
    }

    private var title: String {
        target.path.split(separator: "/").last.map(String.init) ?? "漫画"
    }

    private func load() async {
        guard let client = model.apiClient else {
            errorText = "数据源未就绪"
            loading = false
            return
        }
        loading = true
        errorText = ""
        var collected: [RemoteBrowserEntry] = []
        var cursor: String?
        do {
            repeat {
                let page = try await client.remoteChildren(rootId: target.root.id, path: target.path, cursor: cursor)
                collected.append(contentsOf: page.items.filter { $0.isImage })
                cursor = page.nextCursor
            } while cursor != nil && collected.count < 800

            var sources: [ReaderPageSource] = []
            sources.reserveCapacity(collected.count)
            for entry in collected {
                let fallback: URL?
                if let direct = model.alistDirectURL(remotePath: entry.path) {
                    fallback = direct
                } else {
                    fallback = await model.imageSource(assetId: entry.assetId, remotePath: entry.path)?.url
                }
                guard let fallback else { continue }
                sources.append(.remoteAsset(
                    assetId: entry.assetId ?? "",
                    remotePath: entry.path,
                    fallbackURL: fallback
                ))
            }
            pageSources = sources
            if let startPath = target.startPath,
               let idx = collected.firstIndex(where: { $0.path == startPath }) {
                initialIndex = idx
            }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }
}
