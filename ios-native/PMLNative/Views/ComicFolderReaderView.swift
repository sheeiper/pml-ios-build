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

/// 图片文件夹漫画：条漫连续阅读，或单页/双页翻页。
struct ComicFolderReaderView: View {
    let target: ComicFolderTarget

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var pages: [RemoteBrowserEntry] = []
    @State private var urls: [String: URL] = [:]
    @State private var loading = true
    @State private var errorText = ""
    @State private var currentPage = 0
    @State private var mode: ComicReadingMode = .webtoon
    @State private var doubleStart = 0

    var body: some View {
        NavigationStack {
            Group {
                if loading && pages.isEmpty {
                    ProgressView("加载目录…")
                } else if !errorText.isEmpty {
                    ContentUnavailableView(
                        "无法打开漫画",
                        systemImage: "exclamationmark.triangle",
                        description: Text(errorText)
                    )
                } else if pages.isEmpty {
                    ContentUnavailableView("没有图片页", systemImage: "photo.on.rectangle")
                } else if mode == .webtoon {
                    webtoonBody
                } else {
                    pagedBody
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(.hidden, for: .tabBar)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 14) {
                        Menu {
                            Picker("阅读模式", selection: $mode) {
                                ForEach(ComicReadingMode.allCases) { item in
                                    Label(item.rawValue, systemImage: item.symbol).tag(item)
                                }
                            }
                        } label: {
                            Image(systemName: "ellipsis")
                        }
                        Button("完成") { dismiss() }
                    }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if !pages.isEmpty, mode != .webtoon {
                    Text(pageLabel)
                        .font(.caption2.weight(.semibold))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(.ultraThinMaterial, in: Capsule())
                        .padding(.bottom, 6)
                }
            }
            .task { await load() }
            .task(id: currentPage) {
                guard !pages.isEmpty else { return }
                for offset in [1, 2, 3, -1] {
                    let index = currentPage + offset
                    guard pages.indices.contains(index) else { continue }
                    let entry = pages[index]
                    await ensureURL(entry)
                    if let url = resolvedURL(for: entry) {
                        Task {
                            _ = await ImageCache.shared.cachedOrLoad(url, maxPixel: 1600)
                        }
                    }
                }
            }
        }
    }

    private var title: String {
        target.path.split(separator: "/").last.map(String.init) ?? "漫画"
    }

    private var pageLabel: String {
        if mode == .double {
            let end = min(doubleStart + 2, pages.count)
            return "\(doubleStart + 1)-\(end) / \(pages.count)"
        }
        return "\(currentPage + 1) / \(pages.count)"
    }

    private var webtoonBody: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(pages.enumerated()), id: \.element.id) { index, entry in
                        pageView(entry, index: index)
                            .id(entry.path)
                            .onAppear {
                                if currentPage != index { currentPage = index }
                            }
                    }
                }
            }
            .onAppear {
                guard let startPath = target.startPath,
                      let index = pages.firstIndex(where: { $0.path == startPath }) else { return }
                currentPage = index
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                    proxy.scrollTo(startPath, anchor: .top)
                }
            }
        }
    }

    private var pagedBody: some View {
        TabView(selection: mode == .double ? $doubleStart : $currentPage) {
            if mode == .double {
                ForEach(doubleStarts, id: \.self) { start in
                    HStack(spacing: 2) {
                        ForEach(visibleIndexes(from: start), id: \.self) { index in
                            pageView(pages[index], index: index, updatesCursor: false)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                        }
                    }
                    .tag(start)
                }
            } else {
                ForEach(Array(pages.enumerated()), id: \.element.id) { index, entry in
                    pageView(entry, index: index)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .tag(index)
                }
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        .onChange(of: doubleStart) { _, newValue in
            currentPage = newValue
        }
    }

    private var doubleStarts: [Int] {
        stride(from: 0, to: pages.count, by: 2).map { $0 }
    }

    private func visibleIndexes(from start: Int) -> [Int] {
        [start, start + 1].filter { $0 < pages.count }
    }

    private func resolvedURL(for entry: RemoteBrowserEntry) -> URL? {
        if let cached = urls[entry.path] { return cached }
        return model.alistDirectURL(remotePath: entry.path)
    }

    @ViewBuilder
    private func pageView(_ entry: RemoteBrowserEntry, index: Int, updatesCursor: Bool = true) -> some View {
        Group {
            if let url = resolvedURL(for: entry) {
                CachedImageView(url: url, contentMode: .fit, minHeight: 320, maxPixel: 1600)
                    .frame(maxWidth: .infinity)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, minHeight: 320)
            }
        }
        .task { await ensureURL(entry) }
        .onAppear {
            if updatesCursor && currentPage != index {
                currentPage = index
            }
        }
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
            pages = collected
            if mode != .webtoon, let startPath = target.startPath,
               let index = pages.firstIndex(where: { $0.path == startPath }) {
                currentPage = index
            }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }

    private func ensureURL(_ entry: RemoteBrowserEntry) async {
        guard urls[entry.path] == nil else { return }
        if let direct = model.alistDirectURL(remotePath: entry.path) {
            urls[entry.path] = direct
            return
        }
        guard let assetId = entry.assetId else { return }
        if let source = await model.playbackSource(assetId: assetId, remotePath: entry.path) {
            urls[entry.path] = source.url
        }
    }
}
