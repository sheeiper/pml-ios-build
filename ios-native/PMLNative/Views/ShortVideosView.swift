import AVFoundation
import AVKit
import Combine
import SwiftUI
import UIKit

/// 短视频：上下整页翻动，单击暂停/播放，双击快进，底部进度，自动避开悬浮 TabBar。
struct ShortVideosView: View {
    @EnvironmentObject private var model: AppModel

    @State private var items: [LibraryAsset] = []
    @State private var nextCursor: String?
    @State private var currentID: String?
    @State private var loading = false
    @State private var errorText = ""

    @State private var player = AVPlayer()
    @State private var muted = true
    @State private var isPlaying = false
    @State private var progress: Double = 0
    @State private var duration: Double = 0
    @State private var isBuffering = false
    @State private var favorites: Set<String> = []
    @State private var heartBurst = false
    @State private var authorSheet: LibraryAuthor?
    @State private var currentAsset: LibraryAsset?
    @State private var systemPlayer: ShortVideoPlayerItem?
    @State private var directoryTarget: HomeDirectoryTarget?
    @State private var authorFilterId: String?
    @State private var authorFilterName: String?
    @State private var hudIcon: String?
    @State private var hudTask: Task<Void, Never>?
    @State private var timeObserver: Any?

    var body: some View {
        GeometryReader { geo in
            ScrollView(.vertical) {
                LazyVStack(spacing: 0) {
                    ForEach(items) { item in
                        ShortVideoSlide(
                            item: item,
                            player: player,
                            isCurrent: item.id == currentID,
                            isPlaying: isPlaying,
                            muted: muted,
                            onTap: { togglePlayback() },
                            onDoubleTap: { likeCurrent() },
                            onToggleMute: { toggleMute() },
                            onFullscreen: { Task { await openSystemPlayer() } }
                        )
                        .frame(width: geo.size.width, height: geo.size.height)
                        .id(item.id)
                        .clipped()
                        .scrollTransition { content, phase in
                            content.opacity(phase.isIdentity ? 1 : 0)
                        }
                    }

                    if nextCursor != nil {
                        ProgressView().tint(.white).padding(24)
                    }
                }
                .scrollTargetLayout()
            }
            .scrollTargetBehavior(.paging)
            .scrollPosition(id: $currentID)
            .scrollIndicators(.hidden)
            .clipped()
            .background(Color.black)
        }
        .background(Color.black)
        .overlay(alignment: .trailing) {
            VStack(spacing: 18) {
                Button {
                    Task { await openAuthor() }
                } label: {
                    ZStack {
                        Circle().fill(.ultraThinMaterial)
                        Image(systemName: "person.fill")
                            .foregroundStyle(.white)
                    }
                    .frame(width: 48, height: 48)
                }

                Button(action: likeCurrent) {
                    VStack(spacing: 3) {
                        Image(systemName: isLiked ? "heart.fill" : "heart")
                            .font(.system(size: 28))
                            .foregroundStyle(isLiked ? Color.pink : Color.white)
                        Text("\(favorites.count)")
                            .font(.caption2)
                            .foregroundStyle(.white)
                    }
                }

                ShareLink(item: currentAsset?.fileName ?? "短视频") {
                    VStack(spacing: 3) {
                        Image(systemName: "square.and.arrow.up")
                            .font(.system(size: 24))
                            .foregroundStyle(.white)
                        Text("分享").font(.caption2).foregroundStyle(.white)
                    }
                }

                Menu {
                    Button {
                        Task { await openSystemPlayer() }
                    } label: {
                        Label("系统播放器", systemImage: "rectangle.inset.filled.and.person.filled")
                    }
                    Button {
                        Task { await openDirectory() }
                    } label: {
                        Label("跳转所在目录", systemImage: "folder")
                    }
                    Button(action: likeCurrent) {
                        Label(isLiked ? "取消收藏" : "收藏", systemImage: isLiked ? "heart.slash" : "heart")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle.fill")
                        .font(.system(size: 24))
                        .foregroundStyle(.white)
                }

                Image(systemName: "music.note")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 40, height: 40)
                    .background(.ultraThinMaterial, in: Circle())
                    .rotationEffect(.degrees(0))
            }
            .padding(.trailing, 12)
            .padding(.bottom, 110)
        }        .overlay(alignment: .center) {
            if heartBurst {
                Image(systemName: "heart.fill")
                    .font(.system(size: 120))
                    .foregroundStyle(.pink.opacity(0.95))
                    .allowsHitTesting(false)
            }
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
        .overlay(alignment: .topLeading) {            Menu {
                Button {
                    Task { await openSystemPlayer() }
                } label: {
                    Label("系统播放器", systemImage: "rectangle.inset.filled.and.person.filled")
                }
                .disabled(currentAsset == nil)

                Button {
                    Task { await openDirectory() }
                } label: {
                    Label("跳转所在目录", systemImage: "folder")
                }
                .disabled(currentAsset?.rootId == nil)

                if let id = currentAsset?.authorId, let name = currentAsset?.authorName {
                    Button {
                        setFilter(id: id, name: name)
                    } label: {
                        Label("只看作者：\(name)", systemImage: "person.crop.circle")
                    }
                }
                if authorFilterId != nil {
                    Button {
                        clearFilter()
                    } label: {
                        Label("显示全部作者", systemImage: "arrow.counterclockwise")
                    }
                }
            } label: {
                Image(systemName: "ellipsis.circle.fill")
                    .font(.system(size: 22))
                    .foregroundStyle(.white)
                    .padding(10)
                    .background(.ultraThinMaterial, in: Circle())
            }
            .padding(14)
        }
        .sheet(item: $systemPlayer) { item in
            SystemPlayerView(url: item.url)
                .ignoresSafeArea()
        }
        .sheet(item: $directoryTarget) { target in
            NavigationStack {
                DirectoryView(root: target.root, path: target.path)
                    .environmentObject(model)
            }
        }
        .overlay(alignment: .center) {
            if isBuffering {
                ProgressView()
                    .controlSize(.large)
                    .tint(.white)
            } else if let hudIcon {
                Image(systemName: hudIcon)
                    .font(.system(size: 42, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.92))
                    .padding(22)
                    .background(.ultraThinMaterial, in: Circle())
            }
        }
        .overlay {
            if items.isEmpty && !loading && !errorText.isEmpty {
                ContentUnavailableView("短视频不可用", systemImage: "play.slash", description: Text(errorText))
            } else if items.isEmpty && !loading {
                ContentUnavailableView(
                    "还没有短视频",
                    systemImage: "play.rectangle",
                    description: Text("先在「设置」里扫描资料库。仅 mp4 / webm / mov / m4v 会进入短视频。")
                )
            }
        }
        .navigationTitle("短视频")
        .navigationBarTitleDisplayMode(.inline)
        .toolbarColorScheme(.dark, for: .navigationBar)
        .task {
            configureAudioSession()
            addTimeObserver()
            await loadFeed(reset: true)
            if let currentID {
                await activate(currentID)
            }
        }
        .onChange(of: currentID) { _, newValue in
            Task { await activate(newValue) }
        }
        .onDisappear {
            if let timeObserver { player.removeTimeObserver(timeObserver) }
            timeObserver = nil
            player.pause()
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }

    // MARK: - 数据

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
            if let total = player.currentItem?.duration.seconds, total.isFinite {
                duration = total
            }
            isBuffering = player.timeControlStatus == .waitingToPlayAtSpecifiedRate
        }
    }

    private func loadFeed(reset: Bool) async {
        guard let client = model.apiClient, !loading else { return }
        loading = true
        if reset { errorText = "" }
        do {
            let page = try await client.shortVideoFeed(
                cursor: reset ? nil : nextCursor,
                limit: 12,
                authorId: authorFilterId
            )
            if reset {
                items = page.items
                nextCursor = page.nextCursor
                currentID = page.items.first?.id
            } else {
                items.append(contentsOf: page.items)
                nextCursor = page.nextCursor
            }
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loading = false
    }

    @MainActor
    private func activate(_ id: String?) async {
        guard let id,
              let index = items.firstIndex(where: { $0.id == id }) else { return }
        if index >= items.count - 3 && nextCursor != nil {
            Task { await loadFeed(reset: false) }
        }
        guard let asset = items.first(where: { $0.id == id }),
              let source = await model.playbackSource(assetId: id, remotePath: asset.remotePath) else { return }
        guard currentID == id else { return }
        currentAsset = asset
        player.replaceCurrentItem(with: AVPlayerItem(url: source.url))
        player.isMuted = muted
        player.play()
        isPlaying = true
        progress = 0
        duration = 0
    }

    // MARK: - 操作

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

    private func seek(by delta: Double) {
        guard duration > 0 else { return }
        let target = min(max(0, progress + delta), duration)
        player.seek(to: CMTime(seconds: target, preferredTimescale: 600))
        progress = target
        showHUD(delta > 0 ? "goforward.5" : "gobackward.5")
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
                if liked {
                    favorites.insert(asset.id)
                } else {
                    favorites.remove(asset.id)
                }
            }
        }
        withAnimation(.spring(response: 0.3, dampingFraction: 0.6)) {
            heartBurst = true
        }
        Task {
            try? await Task.sleep(nanoseconds: 700_000_000)
            withAnimation(.easeOut(duration: 0.2)) { heartBurst = false }
        }
    }

    @MainActor
    private func openAuthor() async {
        guard let asset = currentAsset,
              let authorId = asset.authorId,
              let client = model.apiClient else { return }
        if let authors = try? await client.libraryAuthors(filter: "all"),
           let author = authors.first(where: { $0.id == authorId }) {
            authorSheet = author
        }
    }
    private func showHUD(_ icon: String) {
        hudIcon = icon
        hudTask?.cancel()
        hudTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 700_000_000)
            if !Task.isCancelled { hudIcon = nil }
        }
    }

    private func setFilter(id: String, name: String) {
        authorFilterId = id
        authorFilterName = name
        Task { await resetAndReload() }
    }

    private func clearFilter() {
        authorFilterId = nil
        authorFilterName = nil
        Task { await resetAndReload() }
    }

    private func resetAndReload() async {
        player.pause()
        player.replaceCurrentItem(with: nil)
        items = []
        currentID = nil
        currentAsset = nil
        isPlaying = false
        await loadFeed(reset: true)
        if let currentID {
            await activate(currentID)
        }
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
    let muted: Bool
    let onTap: () -> Void
    let onDoubleTap: () -> Void
    let onToggleMute: () -> Void
    let onFullscreen: () -> Void

    var body: some View {
        ZStack {
            Color.black

            if isCurrent {
                PlayerLayerContainer(player: player, gravity: .resizeAspectFill)
            }

            VStack {
                Spacer()
                Button(action: onFullscreen) {
                    Label("全屏观看", systemImage: "rectangle.inset.filled.and.person.filled")
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 14)
                        .padding(.vertical, 7)
                        .background(.ultraThinMaterial, in: Capsule())
                }
                .foregroundStyle(.white)
                .padding(.bottom, 12)

                HStack(alignment: .bottom, spacing: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        if let author = item.authorName, !author.isEmpty {
                            Text("@\(author)").font(.subheadline.weight(.semibold))
                        }
                        Text(item.fileName).font(.footnote).lineLimit(2)
                    }
                    Spacer(minLength: 0)
                }
                .foregroundStyle(.white)
                .padding(.horizontal, 18)
                .padding(.bottom, 10)
            }
            .background(
                LinearGradient(colors: [.clear, .black.opacity(0.55)], startPoint: .top, endPoint: .bottom)
            )

            VStack(spacing: 0) {
                Spacer()
                if isCurrent && !isPlaying {
                    Image(systemName: "play.fill")
                        .font(.system(size: 40))
                        .foregroundStyle(.white.opacity(0.85))
                        .padding(.bottom, 90)
                }
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onTap)
        .simultaneousGesture(
            SpatialTapGesture(count: 2).onEnded { _ in
                onDoubleTap()
            }
        )
        .overlay(alignment: .topTrailing) {
            Button(action: onToggleMute) {
                Image(systemName: muted ? "speaker.slash.fill" : "speaker.wave.2.fill")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.white)
                    .padding(10)
                    .background(.ultraThinMaterial, in: Circle())
            }
            .padding(14)
        }
        .overlay(alignment: .bottom) {
            ShortVideoProgressBar(player: player, isCurrent: isCurrent)
                .padding(.horizontal, 14)
                .padding(.bottom, 4)
        }
    }
}

private struct ShortVideoProgressBar: View {
    let player: AVPlayer
    let isCurrent: Bool
    @State private var value: Double = 0
    @State private var timer = Timer.publish(every: 0.4, on: .main, in: .common).autoconnect()

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.white.opacity(0.25))
                Capsule()
                    .fill(Color.white)
                    .frame(width: geo.size.width * CGFloat(min(max(value, 0), 1)))
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { drag in
                        guard isCurrent, let item = player.currentItem else { return }
                        let total = item.duration.seconds
                        guard total.isFinite, total > 0 else { return }
                        let fraction = min(max(drag.location.x / geo.size.width, 0), 1)
                        value = fraction
                        player.seek(to: CMTime(seconds: fraction * total, preferredTimescale: 600))
                    }
            )
        }
        .frame(height: 14)
        .onReceive(timer) { _ in
            guard isCurrent, let item = player.currentItem else { return }
            let total = item.duration.seconds
            guard total.isFinite, total > 0 else { return }
            let current = player.currentTime().seconds
            guard current.isFinite else { return }
            value = current / total
        }
    }
}
