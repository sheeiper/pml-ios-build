import AVFoundation
import AVKit
import SwiftUI
import UIKit

private enum ShortsScope: String, CaseIterable, Identifiable {
    case all
    case unwatched
    case favorites

    var id: String { rawValue }
    var title: String {
        switch self {
        case .all: return "全部"
        case .unwatched: return "未看"
        case .favorites: return "收藏"
        }
    }
}

private enum ShortsDuration: Int, CaseIterable, Identifiable {
    case all = 0
    case short = 30
    case minute = 60

    var id: Int { rawValue }
    var title: String {
        switch self {
        case .all: return "全部时长"
        case .short: return "30 秒内"
        case .minute: return "60 秒内"
        }
    }
}

/// 短视频：上下整页翻动，预加载下一条，双击暂停，长按 2x，横滑 seek。
/// 页面保持简洁：顶部筛选/静音/更多，右侧头像/收藏/连播，底部作者/标题/进度。
struct ShortVideosView: View {
    @EnvironmentObject private var model: AppModel

    @State private var items: [LibraryAsset] = []
    @State private var nextCursor: String?
    @State private var currentID: String?
    @State private var loading = false
    @State private var loadingMore = false
    @State private var errorText = ""

    @State private var player = AVPlayer()
    @AppStorage("pml.shortVideo.muted") private var muted = true
    @State private var isPlaying = false
    @State private var progress: Double = 0
    @State private var duration: Double = 0
    @State private var isBuffering = false
    @State private var favorites: Set<String> = []
    @State private var seenIDs: Set<String> = []
    @State private var currentAsset: LibraryAsset?

    @State private var scope: ShortsScope = .all
    @State private var durationFilter: ShortsDuration = .all
    @State private var authorFilterId: String?
    @State private var authorFilterName: String?
    @State private var isBinge = false
    @State private var bingeName = ""

    @State private var preloadedID: String?
    @State private var preloadedItem: AVPlayerItem?
    @State private var activateGeneration = 0
    @State private var retriedDirectID: String?
    @State private var lastSavedID: String?
    @State private var lastSavedAt: Double = 0
    @State private var restoringProgressID: String?

    @State private var authorSheet: LibraryAuthor?
    @State private var systemPlayer: ShortVideoPlayerItem?
    @State private var directoryTarget: HomeDirectoryTarget?
    @State private var hudIcon: String?
    @State private var hudTask: Task<Void, Never>?
    @State private var timeObserver: Any?

    @State private var isSeeking = false
    @State private var seekResume = false
    @State private var isLongPressing = false

    @ObservedObject private var covers = ThumbnailService.shared

    var body: some View {
        ZStack {
                Color.black.ignoresSafeArea()

                ScrollView(.vertical) {
                    LazyVStack(spacing: 0) {
                        ForEach(items) { item in
                            slide(item: item)
                                .containerRelativeFrame([.horizontal, .vertical])
                        }
                        if loadingMore {
                            ProgressView().tint(.white).padding(24)
                        }
                    }
                    .scrollTargetLayout()
                }
                .scrollTargetBehavior(.paging)
                .scrollPosition(id: $currentID)
                .scrollIndicators(.hidden)
                .background(Color.black)
            }
            .overlay(alignment: .top) { topBar }
            .overlay(alignment: .trailing) { rightRail.padding(.bottom, 104) }
            .overlay(alignment: .bottom) { bottomInfo }
            .overlay(alignment: .center) { centerOverlay }
            .toolbar(.hidden, for: .navigationBar)
            .toolbarColorScheme(.dark, for: .tabBar)
            .task {
                configureAudioSession()
                addTimeObserver()
                await loadFeed(reset: true)
            }
            .onChange(of: currentID) { _, newValue in
                Task {
                    await activate(newValue)
                    await loadMoreIfNeeded(current: newValue)
                }
            }
            .onChange(of: scope) { _, _ in Task { await resetAndReload() } }
            .onChange(of: durationFilter) { _, _ in Task { await resetAndReload() } }
            .onDisappear {
                saveProgress(force: true)
                if let timeObserver { player.removeTimeObserver(timeObserver) }
                timeObserver = nil
                player.pause()
                try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            }
            .sheet(item: $authorSheet) { author in
                NavigationStack {
                    AuthorProfileView(author: author)
                        .toolbar {
                            ToolbarItem(placement: .topBarTrailing) {
                                Button("完成") { authorSheet = nil }
                            }
                        }
                }
                .environmentObject(model)
            }
            .sheet(item: $systemPlayer) { item in
                SystemPlayerView(url: item.url).ignoresSafeArea()
            }
            .sheet(item: $directoryTarget) { target in
                NavigationStack {
                    DirectoryView(root: target.root, path: target.path)
                        .environmentObject(model)
                }
            }
            .overlay {
                if items.isEmpty && !loading && !errorText.isEmpty {
                    ContentUnavailableView("短视频不可用", systemImage: "play.slash", description: Text(errorText))
                } else if items.isEmpty && !loading {
                    ContentUnavailableView("还没有短视频", systemImage: "play.rectangle", description: Text("扫描资料库后，mp4 / mov / m4v / webm 会进入短视频。"))
                }
            }
        .background(Color.black)
        .preferredColorScheme(.dark)
    }

    private func slide(item: LibraryAsset) -> some View {
        ShortVideoSlide(
            item: item,
            player: player,
            isCurrent: item.id == currentID,
            isPlaying: isPlaying && item.id == currentID,
            poster: covers.images[item.thumbnailAssetId ?? item.id],
            isBuffering: isBuffering && item.id == currentID,
            onSingleTap: { togglePlayback() },
            onDoubleTap: { pauseOrResume() },
            onLongPress: { pressing in
                guard item.id == currentID else { return }
                if pressing {
                    isLongPressing = true
                    player.rate = 2
                    showHUD("2x")
                } else if isLongPressing {
                    isLongPressing = false
                    player.rate = 1
                    hudIcon = nil
                }
            }
        )
        .id(item.id)
        .clipped()
    }
    // MARK: - 顶部 / 右侧 / 底部

    private var topBar: some View {
        HStack(spacing: 10) {
            Menu {
                Picker("范围", selection: $scope) {
                    ForEach(ShortsScope.allCases) { item in
                        Text(item.title).tag(item)
                    }
                }
                Divider()
                Picker("时长", selection: $durationFilter) {
                    ForEach(ShortsDuration.allCases) { item in
                        Text(item.title).tag(item)
                    }
                }
                if authorFilterId != nil || isBinge {
                    Divider()
                    Button("退出筛选", systemImage: "xmark") { clearContextFilter() }
                }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "line.3.horizontal.decrease.circle")
                    Text(contextLabel)
                        .lineLimit(1)
                }
                .font(.caption.weight(.semibold))
                .foregroundStyle(.white)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(.black.opacity(0.35), in: Capsule())
            }

            Spacer()

            Button(action: toggleMute) {
                Image(systemName: muted ? "speaker.slash.fill" : "speaker.wave.2.fill")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 36, height: 36)
                    .background(.black.opacity(0.35), in: Circle())
            }

            Menu {
                Button { Task { await openAuthor() } } label: {
                    Label("作者主页", systemImage: "person.crop.circle")
                }
                .disabled(currentAsset?.authorId == nil)
                if let id = currentAsset?.authorId, let name = currentAsset?.authorName {
                    Button {
                        authorFilterId = id
                        authorFilterName = name
                        Task { await resetAndReload() }
                    } label: {
                        Label("只看作者：\(name)", systemImage: "person.crop.circle.badge.checkmark")
                    }
                }
                Button { Task { await openDirectory() } } label: {
                    Label("所在目录", systemImage: "folder")
                }
                .disabled(currentAsset?.rootId == nil)
                Button { Task { await startBinge() } } label: {
                    Label("连播当前目录", systemImage: "rectangle.stack.badge.play")
                }
                .disabled(currentAsset == nil)
                Button { Task { await openSystemPlayer() } } label: {
                    Label("系统播放器", systemImage: "rectangle.inset.filled.and.person.filled")
                }
                .disabled(currentAsset == nil)
                Divider()
                Button { Task { await resetAndReload() } } label: {
                    Label("刷新推荐", systemImage: "arrow.clockwise")
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 36, height: 36)
                    .background(.black.opacity(0.35), in: Circle())
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 8)
    }

    private var rightRail: some View {
        VStack(spacing: 18) {
            Button { Task { await openAuthor() } } label: {
                ZStack {
                    Circle().fill(.ultraThinMaterial)
                    Text(String((currentAsset?.authorName ?? "?").prefix(1)))
                        .font(.headline)
                        .foregroundStyle(.white)
                }
                .frame(width: 46, height: 46)
                .overlay(Circle().stroke(.white.opacity(0.5), lineWidth: 1))
            }

            Button(action: likeCurrent) {
                VStack(spacing: 3) {
                    Image(systemName: isLiked ? "heart.fill" : "heart")
                        .font(.system(size: 28))
                        .foregroundStyle(isLiked ? Color.pink : Color.white)
                    Text("收藏")
                        .font(.caption2)
                        .foregroundStyle(.white.opacity(0.9))
                }
            }

            Button { Task { await startBinge() } } label: {
                VStack(spacing: 3) {
                    Image(systemName: isBinge ? "rectangle.stack.fill" : "rectangle.stack")
                        .font(.system(size: 24))
                        .foregroundStyle(isBinge ? Color.accentColor : Color.white)
                    Text("连播")
                        .font(.caption2)
                        .foregroundStyle(.white.opacity(0.9))
                }
            }
        }
        .padding(.trailing, 14)
    }

    private var bottomInfo: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .bottom) {
                VStack(alignment: .leading, spacing: 3) {
                    if let author = currentAsset?.authorName, !author.isEmpty {
                        Text("@\(author)").font(.subheadline.weight(.semibold))
                    }
                    Text(currentAsset?.fileName ?? "")
                        .font(.footnote)
                        .lineLimit(2)
                }
                Spacer(minLength: 8)
                Text("\(pmlFormatTime(Int64(progress))) / \(pmlFormatTime(Int64(duration)))")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.white.opacity(0.8))
            }
            progressBar
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .padding(.top, 18)
        .background(
            LinearGradient(colors: [.clear, .black.opacity(0.75)], startPoint: .top, endPoint: .bottom)
        )
    }

    private var progressBar: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(.white.opacity(0.25))
                Capsule().fill(.white)
                    .frame(width: geo.size.width * CGFloat(duration > 0 ? min(max(progress / duration, 0), 1) : 0))
            }
            .frame(height: 3)
            .frame(maxHeight: .infinity, alignment: .center)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        guard duration > 0 else { return }
                        if !isSeeking {
                            isSeeking = true
                            seekResume = isPlaying
                            player.pause()
                        }
                        let ratio = min(max(value.location.x / max(1, geo.size.width), 0), 1)
                        progress = ratio * duration
                        player.seek(to: CMTime(seconds: progress, preferredTimescale: 600), toleranceBefore: .init(seconds: 0.4, preferredTimescale: 600), toleranceAfter: .init(seconds: 0.4, preferredTimescale: 600))
                    }
                    .onEnded { _ in
                        isSeeking = false
                        if seekResume { player.play(); isPlaying = true }
                        saveProgress(force: true)
                    }
            )
        }
        .frame(height: 16)
    }

    private var centerOverlay: some View {
        ZStack {
            if isBuffering {
                ProgressView().controlSize(.large).tint(.white)
            } else if let hudIcon {
                Image(systemName: hudIcon)
                    .font(.system(size: 34, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.92))
                    .padding(20)
                    .background(.ultraThinMaterial, in: Circle())
            } else if !isPlaying, currentID != nil {
                Button(action: togglePlayback) {
                    Image(systemName: "play.fill")
                        .font(.system(size: 34, weight: .semibold))
                        .foregroundStyle(.white)
                        .padding(24)
                        .background(.black.opacity(0.35), in: Circle())
                }
            }
        }
        .allowsHitTesting(!isBuffering)
    }

    // MARK: - 数据

    private var contextLabel: String {
        if isBinge { return "连播 · \(bingeName)" }
        if let authorFilterName { return "作者 · \(authorFilterName)" }
        return scope.title
    }

    /// Business state belongs to the selected core, not device-wide preferences.
    @MainActor
    private func hydrateState(client: PMLAPIClient) async throws {
        async let favoriteIDs = client.favoriteIds()
        async let historyIDs = client.playbackHistoryIds()
        let (canonicalFavorites, canonicalHistory) = try await (favoriteIDs, historyIDs)
        favorites = Set(canonicalFavorites)
        seenIDs = Set(canonicalHistory)
    }

    private func matchesScope(_ asset: LibraryAsset) -> Bool {
        switch scope {
        case .all: return true
        case .unwatched: return !seenIDs.contains(asset.id)
        case .favorites: return favorites.contains(asset.id)
        }
    }

    private func loadFeed(reset: Bool) async {
        guard let client = model.apiClient else { return }
        if reset {
            guard !loading else { return }
            loading = true
            errorText = ""
        } else {
            guard !loadingMore, nextCursor != nil else { return }
            loadingMore = true
        }
        do {
            if reset { try await hydrateState(client: client) }
            var collected = reset ? [] : items
            var cursor = reset ? nil : nextCursor
            var attempts = 0
            repeat {
                let page = try await client.shortVideoFeed(
                    cursor: cursor,
                    limit: 12,
                    authorId: authorFilterId,
                    maxDurationSeconds: durationFilter.rawValue
                )
                let existing = Set(collected.map(\.id))
                let filtered = page.items.filter { matchesScope($0) && !existing.contains($0.id) }
                collected.append(contentsOf: filtered)
                nextCursor = page.nextCursor
                cursor = page.nextCursor
                attempts += 1
                if cursor == nil || collected.count >= 8 || attempts >= 3 { break }
            } while true

            items = collected
            loadingMore = false
            if reset {
                currentID = items.first?.id
            }
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loading = false
    }

    @MainActor
    private func loadMoreIfNeeded(current id: String?) async {
        guard let id, let index = items.firstIndex(where: { $0.id == id }) else { return }
        guard nextCursor != nil, index >= items.count - 2, !loading, !loadingMore else { return }
        await loadFeed(reset: false)
    }

    @MainActor
    private func activate(_ id: String?) async {
        guard let id,
              let asset = items.first(where: { $0.id == id }) else { return }
        if id == currentAsset?.id { return }

        saveProgress(force: true)
        activateGeneration += 1
        let generation = activateGeneration
        currentAsset = asset
        restoringProgressID = id
        lastSavedID = nil
        lastSavedAt = 0
        player.pause()
        isPlaying = false
        progress = 0
        duration = 0

        if let preloadedID, preloadedID == id, let preloadedItem {
            player.replaceCurrentItem(with: preloadedItem)
            self.preloadedID = nil
            self.preloadedItem = nil
        } else {
            guard let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else { return }
            guard generation == activateGeneration, currentID == id else { return }
            player.replaceCurrentItem(with: makeItem(source.url))
        }

        guard generation == activateGeneration else { return }
        observeDirectExpiry(asset: asset, generation: generation)
        player.isMuted = muted
        player.play()
        isPlaying = true
        coversPrefetch()
        Task { await restoreProgressAndMarkSeen(assetId: id, generation: generation) }
        Task { await preloadNext(after: id) }
    }

    @MainActor
    private func observeDirectExpiry(asset: LibraryAsset, generation: Int) {
        let id = asset.id
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 1_200_000_000)
            guard generation == activateGeneration, currentAsset?.id == id else { return }
            guard player.currentItem?.status == .failed, retriedDirectID != id else { return }
            retriedDirectID = id
            guard let source = await model.playbackSource(assetId: id, remotePath: asset.remotePath),
                  source.label?.contains("直连") == true else { return }
            guard generation == activateGeneration, currentAsset?.id == id else { return }
            let resume = progress
            player.replaceCurrentItem(with: makeItem(source.url))
            player.isMuted = muted
            player.play()
            if resume > 1 {
                await player.seek(to: CMTime(seconds: resume, preferredTimescale: 600))
            }
        }
    }

    private func preloadNext(after id: String) async {
        guard let index = items.firstIndex(where: { $0.id == id }),
              items.indices.contains(index + 1) else { return }
        let next = items[index + 1]
        guard preloadedID != next.id,
              let source = await model.playbackSource(assetId: next.id, remotePath: next.remotePath) else { return }
        guard currentID == id else { return }
        let item = makeItem(source.url)
        item.preferredForwardBufferDuration = 2
        preloadedID = next.id
        preloadedItem = item
    }

    private func coversPrefetch() {
        guard let client = model.apiClient else { return }
        let ids = items.prefix(8).compactMap { $0.thumbnailAssetId ?? $0.id }
        covers.prefetch(assetIds: Array(ids), client: client)
    }

    private func makeItem(_ url: URL) -> AVPlayerItem {
        let headers = model.playbackHeaders(for: url)
        guard !headers.isEmpty else { return AVPlayerItem(url: url) }
        let asset = AVURLAsset(url: url, options: ["AVURLAssetHTTPHeaderFieldsKey": headers])
        return AVPlayerItem(asset: asset)
    }

    @MainActor
    private func restoreProgressAndMarkSeen(assetId: String, generation: Int) async {
        var loadedProgress = false
        defer {
            if loadedProgress, activateGeneration == generation, restoringProgressID == assetId {
                restoringProgressID = nil
            }
        }
        guard let client = model.apiClient else { return }
        do {
            // Fetch before writing: never replace an existing resume position
            // with zero merely because a clip has just become visible.
            let saved = try await client.playbackProgress(assetId: assetId)
            loadedProgress = true
            guard activateGeneration == generation, currentID == assetId else { return }
            if saved > 2 {
                try await Task.sleep(nanoseconds: 350_000_000)
                guard activateGeneration == generation, currentID == assetId else { return }
                _ = await player.seek(
                    to: CMTime(seconds: saved, preferredTimescale: 600),
                    toleranceBefore: .init(seconds: 0.5, preferredTimescale: 600),
                    toleranceAfter: .init(seconds: 0.5, preferredTimescale: 600)
                )
                guard activateGeneration == generation, currentID == assetId else { return }
                progress = saved
            }
            let current = player.currentTime().seconds
            let position = max(saved, current.isFinite ? max(0, current) : 0)
            try await client.setPlaybackProgress(assetId: assetId, seconds: position)
            seenIDs.insert(assetId)
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
    }

    private func saveProgress(force: Bool = false) {
        guard let asset = currentAsset, !isSeeking,
              restoringProgressID != asset.id else { return }
        let seconds = player.currentTime().seconds
        guard seconds.isFinite, seconds > 0 else { return }
        if !force, lastSavedID == asset.id, abs(seconds - lastSavedAt) < 2 { return }
        lastSavedAt = seconds
        lastSavedID = asset.id
        guard let client = model.apiClient else { return }
        Task {
            do {
                try await client.setPlaybackProgress(assetId: asset.id, seconds: seconds)
                seenIDs.insert(asset.id)
            } catch {
                if !pmlIsCancellation(error) { errorText = error.localizedDescription }
            }
        }
    }

    private func resetAndReload() async {
        saveProgress(force: true)
        activateGeneration += 1
        restoringProgressID = nil
        player.pause()
        player.replaceCurrentItem(with: nil)
        preloadedID = nil
        preloadedItem = nil
        items = []
        currentID = nil
        currentAsset = nil
        isPlaying = false
        isBinge = false
        bingeName = ""
        await loadFeed(reset: true)
    }

    private func togglePlayback() {
        if player.timeControlStatus == .playing {
            player.pause()
            isPlaying = false
            showHUD("pause.fill")
        } else {
            player.play()
            isPlaying = true
            showHUD("play.fill")
        }
    }

    private func pauseOrResume() {
        if player.timeControlStatus == .playing {
            player.pause()
            isPlaying = false
            showHUD("pause.fill")
        } else {
            player.play()
            isPlaying = true
            showHUD("play.fill")
        }
    }

    private func toggleMute() {
        muted.toggle()
        player.isMuted = muted
        showHUD(muted ? "speaker.slash.fill" : "speaker.wave.2.fill")
    }

    private var isLiked: Bool {
        guard let id = currentAsset?.id else { return false }
        return favorites.contains(id)
    }

    private func likeCurrent() {
        guard let asset = currentAsset, let client = model.apiClient else { return }
        Task {
            if let liked = try? await client.toggleFavorite(assetId: asset.id) {
                if liked { favorites.insert(asset.id) } else { favorites.remove(asset.id) }
            }
        }
        showHUD("heart.fill")
    }

    @MainActor
    private func openAuthor() async {
        guard let asset = currentAsset, let authorId = asset.authorId,
              let client = model.apiClient else { return }
        if let authors = try? await client.libraryAuthors(filter: "all"),
           let author = authors.first(where: { $0.id == authorId }) {
            authorSheet = author
        }
    }

    @MainActor
    private func startBinge() async {
        guard let client = model.apiClient, let asset = currentAsset,
              let all = try? await client.libraryAssets(filter: "videos", limit: 500) else { return }
        let directory = all
            .filter { $0.parentPath == asset.parentPath }
            .sorted { $0.fileName.localizedStandardCompare($1.fileName) == .orderedAscending }
        guard !directory.isEmpty else { return }
        items = directory
        isBinge = true
        bingeName = String((asset.parentPath as NSString).lastPathComponent)
        authorFilterId = nil
        authorFilterName = nil
        currentID = asset.id
        await activate(asset.id)
    }

    private func clearContextFilter() {
        isBinge = false
        bingeName = ""
        authorFilterId = nil
        authorFilterName = nil
        Task { await resetAndReload() }
    }

    @MainActor
    private func openSystemPlayer() async {
        guard let asset = currentAsset,
              let source = await model.playbackSource(assetId: asset.id, remotePath: asset.remotePath) else { return }
        systemPlayer = ShortVideoPlayerItem(url: source.url, title: asset.fileName)
    }

    @MainActor
    private func openDirectory() async {
        guard let client = model.apiClient, let asset = currentAsset, let rootId = asset.rootId,
              let roots = try? await client.libraryRoots(),
              let root = roots.first(where: { $0.id == rootId }) else { return }
        directoryTarget = HomeDirectoryTarget(
            root: root,
            path: asset.parentPath.isEmpty ? root.remotePath : asset.parentPath
        )
    }

    private func showHUD(_ icon: String) {
        hudIcon = icon
        hudTask?.cancel()
        hudTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 700_000_000)
            if !Task.isCancelled { hudIcon = nil }
        }
    }

    private func configureAudioSession() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .moviePlayback, options: [])
        try? session.setActive(true)
    }

    private func addTimeObserver() {
        guard timeObserver == nil else { return }
        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(seconds: 0.25, preferredTimescale: 600),
            queue: .main
        ) { time in
            let seconds = time.seconds
            if seconds.isFinite { progress = seconds }
            if let total = player.currentItem?.duration.seconds, total.isFinite { duration = total }
            isBuffering = player.timeControlStatus == .waitingToPlayAtSpecifiedRate
            if Int(seconds) % 2 == 0 { saveProgress() }
        }
    }
}

private struct ShortVideoPlayerItem: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
}

private struct SystemPlayerView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> AVPlayerViewController {
        let controller = AVPlayerViewController()
        let player = AVPlayer(url: url)
        controller.player = player
        player.play()
        return controller
    }

    func updateUIViewController(_ uiViewController: AVPlayerViewController, context: Context) {}
}

private struct ShortVideoSlide: View {
    let item: LibraryAsset
    let player: AVPlayer
    let isCurrent: Bool
    let isPlaying: Bool
    let poster: UIImage?
    let isBuffering: Bool
    let onSingleTap: () -> Void
    let onDoubleTap: () -> Void
    let onLongPress: (Bool) -> Void

    var body: some View {
        ZStack {
            Color.black
            if isCurrent {
                PlayerLayerContainer(player: player, gravity: .resizeAspectFill)
                if isBuffering || !isPlaying, let poster {
                    Image(uiImage: poster)
                        .resizable()
                        .scaledToFill()
                        .allowsHitTesting(false)
                }
            } else if let poster {
                Image(uiImage: poster)
                    .resizable()
                    .scaledToFill()
                    .allowsHitTesting(false)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { onSingleTap() }
        .simultaneousGesture(
            SpatialTapGesture(count: 2).onEnded { _ in onDoubleTap() }
        )
        .onLongPressGesture(minimumDuration: 0.35, pressing: { pressing in
            onLongPress(pressing)
        }, perform: {})
    }
}
