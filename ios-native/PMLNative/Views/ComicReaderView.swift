import SwiftUI

struct ComicArchiveTarget: Identifiable {
    let id: String
    let title: String
}

/// CBZ / ZIP 漫画阅读：条漫式连续滚动，页面图片走本地 `/api/comic-pages` 路由。
struct ComicReaderView: View {
    let target: ComicArchiveTarget

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var pages: [ComicArchivePageInfo] = []
    @State private var loading = true
    @State private var errorText = ""
    @State private var currentPage = 0
    @State private var coverMessage = ""
    @State private var mode: ComicReadingMode = .webtoon
    @State private var pagedOffset = 0

    var body: some View {
        NavigationStack {
            Group {
                if loading {
                    ProgressView("解析漫画压缩包…")
                } else if !errorText.isEmpty {
                    ContentUnavailableView(
                        "无法打开漫画",
                        systemImage: "exclamationmark.triangle",
                        description: Text(errorText)
                    )
                } else if mode == .webtoon {
                    webtoon
                } else {
                    paged
                }
            }
            .navigationTitle(target.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    if !pages.isEmpty {
                        Text(pageLabel)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 14) {
                        Menu {
                            Picker("阅读模式", selection: $mode) {
                                ForEach(ComicReadingMode.allCases) { item in
                                    Label(item.rawValue, systemImage: item.symbol).tag(item)
                                }
                            }
                        } label: {
                            Image(systemName: mode.symbol)
                        }
                        Button {
                            Task { await setAsCover() }
                        } label: {
                            Image(systemName: "photo.badge.checkmark")
                        }
                        .disabled(pages.isEmpty)
                        Button("完成") { dismiss() }
                    }
                }
            }
            .task { await load() }
            .task(id: currentPage) {
                guard !pages.isEmpty else { return }
                let next = currentPage + 1
                if pages.contains(where: { $0.index == next }) {
                    ComicDirectStore.shared.prefetch(assetId: target.id, index: next, model: model)
                }
            }
            .onChange(of: mode) { _, _ in
                if let offset = pages.firstIndex(where: { $0.index == currentPage }) {
                    pagedOffset = offset
                }
            }
            .alert(
                "封面",
                isPresented: Binding(
                    get: { !coverMessage.isEmpty },
                    set: { if !$0 { coverMessage = "" } }
                )
            ) {
                Button("好") {}
            } message: {
                Text(coverMessage)
            }
        }
    }

    private var pageLabel: String {
        if mode == .double {
            let start = pagedOffset
            let end = min(start + 2, pages.count)
            return "\(start + 1)-\(end) / \(pages.count)"
        }
        return "\(currentPage + 1) / \(pages.count)"
    }

    private var webtoon: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(pages, id: \.index) { page in
                    pageView(page)
                        .frame(maxWidth: .infinity)
                }
            }
        }
        .background(Color.black.opacity(0.04))
    }

    private var paged: some View {
        TabView(selection: $pagedOffset) {
            if mode == .double {
                ForEach(stride(from: 0, to: pages.count, by: 2).map { $0 }, id: \.self) { start in
                    HStack(spacing: 2) {
                        ForEach([start, start + 1].filter { $0 < pages.count }, id: \.self) { offset in
                            pageView(pages[offset])
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                        }
                    }
                    .tag(start)
                }
            } else {
                ForEach(Array(pages.enumerated()), id: \.element.index) { offset, page in
                    pageView(page)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .tag(offset)
                }
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        .onChange(of: pagedOffset) { _, newValue in
            guard pages.indices.contains(newValue) else { return }
            currentPage = pages[newValue].index
        }
    }

    private func pageView(_ page: ComicArchivePageInfo) -> some View {
        ComicDirectPageView(assetId: target.id, page: page)
            .onAppear {
                if page.index > currentPage {
                    currentPage = page.index
                }
            }
    }

    private func pageURL(_ index: Int) -> URL? {
        model.apiClient?.comicPageURL(assetId: target.id, pageIndex: index)
    }

    private func setAsCover() async {
        guard let client = model.apiClient, !pages.isEmpty else { return }
        do {
            _ = try await client.setComicCoverPage(assetId: target.id, pageIndex: currentPage)
            coverMessage = "已把第 \(currentPage + 1) 页设为封面"
        } catch {
            coverMessage = error.localizedDescription
        }
    }

    private func load() async {        guard let client = model.apiClient else {
            errorText = "数据源未就绪"
            loading = false
            return
        }
        loading = true
        errorText = ""
        do {
            let info = try await client.inspectComicArchive(assetId: target.id)
            pages = info.pages
            if pages.isEmpty {
                errorText = "压缩包里没有可显示的图片页"
            }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }
}
