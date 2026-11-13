import SwiftUI
import UIKit

struct ImageViewerContext: Identifiable {
    let id = UUID()
    let entries: [RemoteBrowserEntry]
    let startIndex: Int
}

/// 全屏图片查看：横向分页 + 捏合缩放，或条漫纵向滚动。
struct ImageViewer: View {
    let context: ImageViewerContext

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var index: Int
    @State private var currentID: String?
    @State private var urls: [String: URL] = [:]
    @State private var favorites: Set<String> = []
    @State private var webtoonMode = false

    init(context: ImageViewerContext) {
        self.context = context
        let start = min(max(0, context.startIndex), max(0, context.entries.count - 1))
        _index = State(initialValue: start)
        _currentID = State(initialValue: context.entries.indices.contains(start) ? context.entries[start].id : nil)
    }

    var body: some View {
        NavigationStack {
            Group {
                if webtoonMode {
                    webtoon
                } else {
                    pager
                }
            }
            .background(Color.black.ignoresSafeArea())
            .navigationTitle(currentTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("完成") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 16) {
                        Button {
                            webtoonMode.toggle()
                        } label: {
                            Image(systemName: webtoonMode ? "rectangle.portrait" : "rectangle.split.3x1")
                        }
                        Button {
                            Task { await toggleFavorite() }
                        } label: {
                            Image(systemName: isFavorite ? "heart.fill" : "heart")
                        }
                    }
                }
            }
            .task {
                await loadFavorites()
                await prefetch(around: index)
            }
            .onChange(of: currentID) { _, newValue in
                guard let newValue, let idx = context.entries.firstIndex(where: { $0.id == newValue }) else { return }
                if index != idx { index = idx }
                Task { await prefetch(around: idx) }
            }
        }
    }

    // MARK: - 分页模式

    private var pager: some View {
        GeometryReader { geo in
            ScrollViewReader { proxy in
                ScrollView(.horizontal) {
                    LazyHStack(spacing: 0) {
                        ForEach(context.entries) { entry in
                            ZoomableImagePage(url: resolvedURL(for: entry), posterAssetId: entry.assetId)
                                .frame(width: geo.size.width, height: geo.size.height)
                                .id(entry.id)
                                .task { await load(entry) }
                        }
                    }
                    .scrollTargetLayout()
                }
                .scrollTargetBehavior(.paging)
                .scrollPosition(id: $currentID)
                .scrollIndicators(.hidden)
                .onAppear {
                    guard let target = currentEntry?.id else { return }
                    DispatchQueue.main.async {
                        proxy.scrollTo(target, anchor: .center)
                    }
                }
            }
        }
    }

    // MARK: - 条漫模式

    private var webtoon: some View {
        ScrollView {
            LazyVStack(spacing: 0) {
                ForEach(context.entries) { entry in
                    CachedImageView(url: resolvedURL(for: entry), contentMode: .fit, minHeight: 360, maxPixel: 1800)
                        .frame(maxWidth: .infinity)
                        .task { await load(entry) }
                        .onAppear {
                            if currentID != entry.id {
                                currentID = entry.id
                            }
                        }
                }
            }
        }
    }

    private var currentEntry: RemoteBrowserEntry? {
        context.entries.indices.contains(index) ? context.entries[index] : nil
    }

    private var currentTitle: String {
        guard let entry = currentEntry else { return "" }
        return "\(index + 1)/\(context.entries.count) · \(entry.name)"
    }

    private var isFavorite: Bool {
        guard let assetId = currentEntry?.assetId else { return false }
        return favorites.contains(assetId)
    }

    /// 同步计算 AList `/d` 直链，首帧即可发起图片加载，无需等待二次状态刷新。
    private func resolvedURL(for entry: RemoteBrowserEntry) -> URL? {
        if let cached = urls[entry.id] { return cached }
        return model.alistDirectURL(remotePath: entry.path)
    }

    @MainActor
    private func load(_ entry: RemoteBrowserEntry) async -> URL? {
        if let existing = resolvedURL(for: entry) {
            if urls[entry.id] == nil {
                urls[entry.id] = existing
            }
            return existing
        }
        guard let assetId = entry.assetId,
              let source = await model.playbackSource(assetId: assetId, remotePath: entry.path) else { return nil }
        urls[entry.id] = source.url
        return source.url
    }

    @MainActor
    private func prefetch(around center: Int) async {
        let pixelLimit: CGFloat = webtoonMode ? 1800 : 2400
        for offset in [0, 1, -1, 2, 3, -2] {
            let idx = center + offset
            guard context.entries.indices.contains(idx) else { continue }
            let entry = context.entries[idx]
            if let url = await load(entry) {
                Task {
                    _ = await ImageCache.shared.cachedOrLoad(url, maxPixel: pixelLimit)
                }
            }
        }
    }

    @MainActor
    private func loadFavorites() async {
        guard let client = model.apiClient else { return }
        if let ids = try? await client.favoriteIds() {
            favorites = Set(ids)
        }
    }

    @MainActor
    private func toggleFavorite() async {
        guard let client = model.apiClient, let assetId = currentEntry?.assetId else { return }
        if let nowFavorite = try? await client.toggleFavorite(assetId: assetId) {
            if nowFavorite {
                favorites.insert(assetId)
            } else {
                favorites.remove(assetId)
            }
        }
    }
}

/// 单页图片：纯 SwiftUI 实现，捏合缩放 / 拖动平移 / 双击缩放，布局不依赖 UIKit frame。
private struct ZoomableImagePage: View {
    let url: URL?
    var posterAssetId: String? = nil

    @State private var image: UIImage?

    private var displayImage: UIImage? {
        if let image { return image }
        guard let url else { return nil }
        return ImageCache.shared.cached(url, maxPixel: 2400)
    }

    var body: some View {
        ZStack {
            Color.black
            if let current = displayImage {
                ZoomScrollRepresentable(image: current)
            } else if let posterAssetId, let poster = ThumbnailService.shared.cachedImage(assetId: posterAssetId) {
                Image(uiImage: poster)
                    .resizable()
                    .scaledToFit()
                ProgressView().tint(.white)
            } else {
                ProgressView().tint(.white)
            }
        }
        .task(id: url) {
            guard let url else { return }
            if let hit = ImageCache.shared.cached(url, maxPixel: 2400) {
                image = hit
                return
            }
            image = await ImageCache.shared.cachedOrLoad(url, maxPixel: 2400)
        }
    }
}

private struct ZoomScrollRepresentable: UIViewRepresentable {
    let image: UIImage

    func makeUIView(context: Context) -> ZoomScrollView {
        let view = ZoomScrollView()
        view.setImage(image)
        return view
    }

    func updateUIView(_ view: ZoomScrollView, context: Context) {
        view.setImage(image)
    }
}

/// UIKit 缩放容器：滚动/缩放交给 UIScrollView，避免 SwiftUI scaleEffect 卡顿。
final class ZoomScrollView: UIScrollView, UIScrollViewDelegate {
    private let imageView = UIImageView()
    private var lastImage: UIImage?

    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func setup() {
        delegate = self
        minimumZoomScale = 1
        maximumZoomScale = 5
        bouncesZoom = true
        backgroundColor = .black
        showsVerticalScrollIndicator = false
        showsHorizontalScrollIndicator = false
        contentInsetAdjustmentBehavior = .never
        decelerationRate = .fast

        imageView.contentMode = .scaleAspectFit
        imageView.clipsToBounds = true
        addSubview(imageView)

        let doubleTap = UITapGestureRecognizer(target: self, action: #selector(handleDoubleTap(_:)))
        doubleTap.numberOfTapsRequired = 2
        addGestureRecognizer(doubleTap)
    }

    func setImage(_ image: UIImage) {
        guard image !== lastImage else { return }
        lastImage = image
        imageView.image = image
        setZoomScale(1, animated: false)
        isScrollEnabled = false
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        if zoomScale <= 1.01 {
            imageView.frame = bounds
            contentSize = bounds.size
        } else {
            contentSize = imageView.frame.size
        }
    }

    func viewForZooming(in scrollView: UIScrollView) -> UIView? {
        imageView
    }

    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        isScrollEnabled = zoomScale > 1.01
    }

    @objc private func handleDoubleTap(_ gesture: UITapGestureRecognizer) {
        if zoomScale > 1.01 {
            setZoomScale(1, animated: true)
            return
        }
        let point = gesture.location(in: imageView)
        let width = bounds.width / 2.5
        let height = bounds.height / 2.5
        zoom(
            to: CGRect(x: point.x - width / 2, y: point.y - height / 2, width: width, height: height),
            animated: true
        )
    }
}