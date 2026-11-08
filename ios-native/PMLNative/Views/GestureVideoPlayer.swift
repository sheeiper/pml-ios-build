import AVFoundation
import AVKit
import SwiftUI
import UIKit

struct VideoPlaylistItem: Identifiable {
    let id: String
    let title: String
    let assetId: String?
    var remotePath: String? = nil
}

/// 原生视频播放器：双击 ±10s、长按 2x、上下滑亮度/音量、横滑进度、倍速、续播、上下集。
struct GestureVideoPlayerSheet: View {
    var startAt: Double = 0
    @State private var playlist: [VideoPlaylistItem]

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var player = AVPlayer()
    @State private var currentURL: URL
    @State private var currentTitle: String
    @State private var currentAssetId: String?
    @State private var currentIndex: Int?
    @State private var accelLabel: String?
    @State private var currentFallbackURL: URL?
    @State private var currentSourceLabel: String?
    @State private var usedFallback = false

    @State private var isPlaying = false
    @State private var currentTime: Double = 0
    @State private var duration: Double = 0
    @State private var rate: Float = 1
    @State private var muted = false
    @State private var volume: Float = 1
    @State private var controlsVisible = true
    @State private var controlsTask: Task<Void, Never>?

    @State private var resumeSeconds: Double = 0
    @State private var resumeVisible = false
    @State private var resumeChecked = false
    @State private var hudText: String?
    @State private var hudTask: Task<Void, Never>?
    @State private var timeObserver: Any?
    @State private var lastSavedAt: Double = 0

    @State private var dragMode: DragMode?
    @State private var pressTask: Task<Void, Never>?
    @State private var isBoosting = false
    @State private var dragBaseTime: Double = 0
    @State private var seekWasPlaying = false
    @State private var lastSeekHapticSecond: Double = 0
    @State private var dragBaseBrightness: CGFloat = 1
    @State private var dragBaseVolume: Float = 1
    @State private var seekPreview: Double?
    @State private var isScrubbing = false
    @State private var pendingSeek: Double?
    @State private var isBuffering = false
    @State private var didReachEnd = false
    @State private var seekGeneration = 0
    @State private var seekTask: Task<Void, Never>?
    @State private var seekRequestID = 0
    @State private var seekWatchdog: Task<Void, Never>?
    @State private var isRecovering = false
    @State private var stallRecoveryTask: Task<Void, Never>?
    @State private var subtitleTracks: [SubtitleTrack] = []
    @State private var subtitleCues: [SubtitleCue] = []
    @State private var subtitleTitle: String?
    @State private var subtitleOffset: Double = 0
    @State private var backgroundObserver: NSObjectProtocol?
    @State private var showSystemPlayer = false
    @State private var isLocked = false
    @State private var showPlaylist = false
    @State private var showResumeToast = false
    @State private var resumeToastSeconds: Double = 0
    @State private var videoGravity: AVLayerVideoGravity = .resizeAspect
    @AppStorage("pml.player.scrubRange") private var scrubRangeSetting: Double = 180
    @State private var baseBrightness: CGFloat = UIScreen.main.brightness
    @State private var poster: UIImage?
    @ObservedObject private var thumbService = ThumbnailService.shared

    private enum DragMode { case seek, brightness, volume }

    @State private var endObserver: NSObjectProtocol?

    init(
        url: URL,
        title: String,
        assetId: String? = nil,
        startAt: Double = 0,
        playlist: [VideoPlaylistItem] = [],
        playlistIndex: Int? = nil,
        fallbackURL: URL? = nil,
        sourceLabel: String? = nil
    ) {
        self.startAt = startAt
        _playlist = State(initialValue: playlist)
        _currentURL = State(initialValue: url)
        _currentTitle = State(initialValue: title)
        _currentAssetId = State(initialValue: assetId)
        _currentIndex = State(initialValue: playlistIndex)
        _currentFallbackURL = State(initialValue: fallbackURL)
        _currentSourceLabel = State(initialValue: sourceLabel)
        _accelLabel = State(initialValue: sourceLabel)
    }

    var body: some View {
        GeometryReader { geo in
            ZStack {
                Color.black.ignoresSafeArea()
                PlayerLayerContainer(player: player, gravity: videoGravity)
                    .ignoresSafeArea()

                if let poster, duration <= 0 {
                    Image(uiImage: poster)
                        .resizable()
                        .scaledToFit()
                        .ignoresSafeArea()
                        .allowsHitTesting(false)
                }

                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture { toggleControls() }
                    .simultaneousGesture(
                        SpatialTapGesture(count: 2).onEnded { value in
                            let width = geo.size.width
                            if value.location.x < width * 0.35 {
                                seek(by: -10)
                            } else if value.location.x > width * 0.65 {
                                seek(by: 10)
                            } else {
                                togglePlay()
                                showHUD(isPlaying ? "继续播放" : "已暂停", seconds: 1)
                            }
                        }
                    )
                    .simultaneousGesture(
                        DragGesture(minimumDistance: 0)
                            .onChanged { handleDrag($0, size: geo.size) }
                            .onEnded { endDrag($0, size: geo.size) }
                    )

                if let hudText {
                    Text(hudText)
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .background(.ultraThinMaterial, in: Capsule())
                }

                if dragMode == .seek, let preview = seekPreview {
                    VStack(spacing: 4) {
                        Text("\(preview >= currentTime ? "+" : "")\(Int(preview - currentTime))s")
                            .font(.system(size: 26, weight: .semibold, design: .rounded))
                            .monospacedDigit()
                        Text("\(pmlFormatTime(Int64(min(max(0, preview), max(0, duration))))) / \(pmlFormatTime(Int64(duration)))")
                            .font(.caption)
                            .monospacedDigit()
                    }
                    .foregroundStyle(.white)
                    .padding(.horizontal, 22)
                    .padding(.vertical, 12)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                }
                if isBuffering {
                    ProgressView()
                        .controlSize(.large)
                        .tint(.white)
                }

                if resumeVisible {
                    resumeCard
                }

                if let subtitle = currentSubtitleText, !isLocked {
                    Text(subtitle)
                        .font(.system(size: 17, weight: .semibold))
                        .multilineTextAlignment(.center)
                        .foregroundStyle(.white)
                        .shadow(color: .black.opacity(0.9), radius: 3, x: 0, y: 1)
                        .padding(.horizontal, 24)
                        .padding(.bottom, controlsVisible ? 118 : 52)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
                        .allowsHitTesting(false)
                }

                if controlsVisible && !isLocked {
                    controls
                }
            }
            .overlay(alignment: .top) {
                if controlsVisible && !isLocked {
                    HStack(spacing: 12) {
                        Button { dismiss() } label: {
                            Image(systemName: "chevron.left")
                                .font(.system(size: 16, weight: .semibold))
                                .frame(width: 36, height: 36)
                                .contentShape(Rectangle())
                        }
                        Text(currentTitle)
                            .font(.subheadline.weight(.semibold))
                            .lineLimit(1)
                        Spacer(minLength: 8)
                        if let accelLabel {
                            Text(accelLabel)
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(accelLabel.contains("直连") ? Color.green : Color.white.opacity(0.85))
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(.ultraThinMaterial, in: Capsule())
                        }
                        Menu {
                            Button { showPlaylist = true } label: {
                                Label("播放列表", systemImage: "list.bullet")
                            }
                            Menu {
                                Button("关闭字幕") {
                                    subtitleCues = []
                                    subtitleTitle = nil
                                }
                                ForEach(subtitleTracks) { track in
                                    Button(track.name) {
                                        Task { await applySubtitle(track) }
                                    }
                                }
                                Divider()
                                Button("字幕延迟 +0.5s") { subtitleOffset += 0.5 }
                                Button("字幕延迟 -0.5s") { subtitleOffset -= 0.5 }
                            } label: {
                                Label("字幕", systemImage: "captions.bubble")
                            }
                            Picker("滑动灵敏度", selection: $scrubRangeSetting) {
                                Text("低").tag(90.0)
                                Text("标准").tag(180.0)
                                Text("高").tag(300.0)
                            }
                            Button { toggleAspect() } label: {
                                Label(videoGravity == .resizeAspect ? "填充画面" : "适配画面", systemImage: "aspectratio")
                            }
                            Button {
                                muted.toggle()
                                player.isMuted = muted
                            } label: {
                                Label(muted ? "取消静音" : "静音", systemImage: muted ? "speaker.wave.2" : "speaker.slash")
                            }
                            Button { showSystemPlayer = true } label: {
                                Label("系统播放器", systemImage: "rectangle.inset.filled.and.person.filled")
                            }
                        } label: {
                            Image(systemName: "ellipsis")
                                .font(.system(size: 16, weight: .semibold))
                                .frame(width: 36, height: 36)
                                .contentShape(Rectangle())
                        }
                    }
                    .foregroundStyle(.white)
                    .padding(.horizontal, 18)
                    .padding(.top, 52)
                }
            }
            .overlay(alignment: .leading) {
                Button {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        isLocked.toggle()
                        if isLocked { controlsVisible = false }
                    }
                } label: {
                    Image(systemName: isLocked ? "lock.fill" : "lock.open")
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(.white)
                        .padding(10)
                        .background(.ultraThinMaterial, in: Circle())
                }
                .padding(.leading, 14)
                .opacity(controlsVisible || isLocked ? 1 : 0)
            }
            .overlay(alignment: .bottom) {
                if showResumeToast {
                    HStack(spacing: 12) {
                        Text("已从上次观看点 (\(pmlFormatTime(Int64(resumeToastSeconds)))) 处播放")
                            .font(.caption)
                        Button("从头开始播放") { seekToStart() }
                            .font(.caption.weight(.semibold))
                    }
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(.ultraThinMaterial, in: Capsule())
                    .padding(.bottom, 130)
                }
            }            .statusBarHidden(true)
        }
        .onAppear(perform: setup)
        .onAppear {
            OrientationController.shared.lock(.allButUpsideDown)
            if currentIndex == nil {
                currentIndex = resolvedIndex
            }
        }
        .onDisappear {
            teardown()
            OrientationController.shared.lock(.portrait)
        }
        .fullScreenCover(isPresented: $showSystemPlayer) {
            SystemPlayerScreen(url: currentURL)
                .ignoresSafeArea()
        }
        .sheet(isPresented: $showPlaylist) {
            playlistSheet
        }
    }

    private var playlistSheet: some View {
        NavigationStack {
            List(playlist.indices, id: \.self) { index in
                Button {
                    Task { await playIndex(index) }
                } label: {
                    HStack {
                        Text(playlist[index].title).lineLimit(1)
                        Spacer()
                        if resolvedIndex == index {
                            Image(systemName: "checkmark").foregroundStyle(.tint)
                        }
                    }
                }
            }
            .navigationTitle("播放列表")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { showPlaylist = false }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    // MARK: - 控制条

    private var controls: some View {
        VStack {
            Spacer()
            VStack(spacing: 10) {
                HStack {
                    Text(pmlFormatTime(Int64(currentTime)))
                    Spacer()
                    if !playlist.isEmpty, let currentIndex {
                        Text("\(currentIndex + 1)/\(playlist.count)")
                    }
                    Spacer()
                    Text(pmlFormatTime(Int64(duration)))
                }
                .font(.caption2)
                .foregroundStyle(.white.opacity(0.85))

                PlayerProgressSlider(
                    value: pendingSeek ?? currentTime,
                    duration: duration,
                    onScrubStart: {
                        if !isScrubbing {
                            isScrubbing = true
                            player.pause()
                        }
                    },
                    onScrub: { seconds in pendingSeek = seconds },
                    onScrubEnd: { seconds in
                        pendingSeek = seconds
                        finishScrub()
                    }
                )

                HStack(spacing: 22) {
                    Button { togglePlay() } label: {
                        Image(systemName: isPlaying ? "pause.fill" : "play.fill")
                            .font(.system(size: 22))
                    }
                    if !playlist.isEmpty {
                        Button {
                            Task { await playRelative(-1) }
                        } label: {
                            Image(systemName: "backward.end.fill")
                        }
                        .disabled(!canGoPrevious)
                        Button {
                            Task { await playRelative(1) }
                        } label: {
                            Image(systemName: "forward.end.fill")
                        }
                        .disabled(!canGoNext)
                    }
                    Spacer()
                    Menu {
                        ForEach([0.5, 1.0, 1.25, 1.5, 2.0], id: \.self) { value in
                            Button(String(format: "%.2gx", value)) { setRate(Float(value)) }
                        }
                    } label: {
                        Text(String(format: "%.2gx", rate))
                            .font(.caption.weight(.semibold))
                    }
                    Button {
                        toggleAspect()
                    } label: {
                        Text(videoGravity == .resizeAspect ? "适配" : "填充")
                            .font(.caption.weight(.semibold))
                    }
                    Button {
                        showPlaylist = true
                    } label: {
                        Image(systemName: "list.bullet")
                    }
                    Button {
                        toggleOrientation()
                    } label: {
                        Image(systemName: "arrow.up.left.and.arrow.down.right")
                    }
                }
                .foregroundStyle(.white)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 24)
            .padding(.top, 14)
            .background(
                LinearGradient(colors: [.clear, .black.opacity(0.7)], startPoint: .top, endPoint: .bottom)
            )
        }
        .transition(.opacity)
    }

    private var resumeCard: some View {
        VStack(spacing: 14) {
            Text("\(currentTitle) · 上次看到 \(pmlFormatTime(Int64(resumeSeconds)))")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.white)
                .multilineTextAlignment(.center)
            HStack(spacing: 12) {
                Button("继续播放") { begin(from: resumeSeconds) }
                    .buttonStyle(.borderedProminent)
                Button("从头开始") { begin(from: 0) }
                    .buttonStyle(.bordered)
                    .tint(.white)
            }
        }
        .padding(20)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
    }

    // MARK: - 播放控制

    private func setup() {
        baseBrightness = UIScreen.main.brightness
        configureAudioSession()
        addTimeObserver()
        loadCurrent(trackResume: true)
        scheduleHideControls()
        if playlist.isEmpty, let assetId = currentAssetId {
            Task { await loadSiblingPlaylist(assetId) }
        }
        if let existing = backgroundObserver {
            NotificationCenter.default.removeObserver(existing)
        }
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didEnterBackgroundNotification,
            object: nil,
            queue: .main
        ) { [self] _ in
            saveProgress()
        }
        if let existing = endObserver {
            NotificationCenter.default.removeObserver(existing)
        }
        endObserver = NotificationCenter.default.addObserver(
            forName: .AVPlayerItemDidPlayToEndTime,
            object: nil,
            queue: .main
        ) { [self] _ in
            guard isPlaying else { return }
            if canGoNext {
                Task { await playRelative(1) }
            } else {
                isPlaying = false
            }
        }
    }

    @MainActor
    private func loadSubtitles() async {
        subtitleTracks = []
        subtitleCues = []
        subtitleTitle = nil
        subtitleOffset = 0
        guard let client = model.apiClient, let assetId = currentAssetId else { return }
        guard let tracks = try? await client.findAssetSubtitles(assetId: assetId), !tracks.isEmpty else { return }
        subtitleTracks = tracks
        if let first = tracks.first {
            await applySubtitle(first)
        }
    }

    @MainActor
    private func applySubtitle(_ track: SubtitleTrack) async {
        guard let client = model.apiClient else { return }
        do {
            let text = try await client.subtitleText(remotePath: track.path)
            subtitleCues = SubtitleParser.parse(text)
            subtitleTitle = track.name
        } catch {
            subtitleCues = []
            subtitleTitle = nil
            showHUD("字幕加载失败", seconds: 2)
        }
    }

    private var currentSubtitleText: String? {
        guard !subtitleCues.isEmpty else { return nil }
        return SubtitleParser.cue(at: currentTime + subtitleOffset, in: subtitleCues)?.text
    }

    private func handlePlaybackEnd() {
        saveProgress()
        if canGoNext {
            Task { await playRelative(1) }
        } else {
            isPlaying = false
        }
    }

    @MainActor
    private func loadSiblingPlaylist(_ assetId: String) async {
        guard let client = model.apiClient else { return }
        guard let siblings = try? await client.listSiblingVideos(assetId: assetId), !siblings.isEmpty else { return }
        let items = siblings.map {
            VideoPlaylistItem(id: $0.id, title: $0.title, assetId: $0.id)
        }
        playlist = items
        currentIndex = items.firstIndex(where: { $0.assetId == assetId })
    }
    @MainActor
    private func waitUntilReady(timeout: TimeInterval = 8) async -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            guard let item = player.currentItem else { return false }
            switch item.status {
            case .readyToPlay:
                return true
            case .failed:
                return false
            default:
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
        }
        return player.currentItem?.status == .readyToPlay
    }

    /// 所有 seek 统一入口：120ms 内合并，只让最后一个请求执行，避免滑块/双击/横滑并发。
    private func requestSeek(to seconds: Double, resume: Bool, debounce: Bool = true) {
        seekRequestID += 1
        let request = seekRequestID
        seekTask?.cancel()
        seekTask = Task { @MainActor in
            if debounce {
                try? await Task.sleep(nanoseconds: 120_000_000)
            }
            guard !Task.isCancelled, request == seekRequestID else { return }
            await performSeek(to: seconds, resume: resume, request: request)
        }
    }

    @MainActor
    private func performSeek(to seconds: Double, resume: Bool, request: Int) async {
        guard let item = player.currentItem, request == seekRequestID else { return }
        if item.status != .readyToPlay {
            guard await waitUntilReady(timeout: 6), request == seekRequestID else { return }
        }
        item.cancelPendingSeeks()
        let total = item.duration.seconds.isFinite ? item.duration.seconds : duration
        let target = min(max(0, seconds), max(0, total))
        let time = CMTime(seconds: target, preferredTimescale: 600)
        let tolerance = CMTime(seconds: 0.4, preferredTimescale: 600)
        // 先挂看门狗：即使 seek 本身卡住，也能重建 item 恢复。
        armSeekWatchdog(target: target, request: request)
        let didSeek = await player.seek(to: time, toleranceBefore: tolerance, toleranceAfter: tolerance)
        guard request == seekRequestID else { return }
        guard didSeek || abs(player.currentTime().seconds - target) < 1.5 else { return }
        currentTime = target
        if resume {
            player.play()
            isPlaying = true
        } else {
            player.pause()
            isPlaying = false
        }
    }

    /// seek 后 4 秒仍没有画面/时间没有前进，就重建 item 并从目标位置恢复。
    private func armSeekWatchdog(target: Double, request: Int) {
        seekWatchdog?.cancel()
        seekWatchdog = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            guard !Task.isCancelled, request == seekRequestID, isPlaying else { return }
            let current = player.currentTime().seconds
            let stalled = player.currentItem?.status == .failed
                || player.timeControlStatus == .waitingToPlayAtSpecifiedRate
                || !current.isFinite
            guard stalled else { return }
            await recoverPlayback(at: target)
        }
    }

    @MainActor
    private func recoverPlayback(at target: Double) async {
        guard !isRecovering else { return }
        isRecovering = true
        defer { isRecovering = false }
        seekRequestID += 1
        seekTask?.cancel()
        var url = currentURL
        if player.currentItem?.status == .failed, let fallback = currentFallbackURL {
            url = fallback
            accelLabel = "服务器中转"
        }
        player.replaceCurrentItem(with: AVPlayerItem(url: url))
        if await waitUntilReady(timeout: 8) {
            requestSeek(to: target, resume: true, debounce: false)
        }
    }

    private func recoverFromStall() {
        guard stallRecoveryTask == nil else { return }
        stallRecoveryTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 7_000_000_000)
            guard !Task.isCancelled, isPlaying,
                  player.timeControlStatus == .waitingToPlayAtSpecifiedRate else {
                stallRecoveryTask = nil
                return
            }
            let target = currentTime
            player.replaceCurrentItem(with: AVPlayerItem(url: currentURL))
            if await waitUntilReady() {
                requestSeek(to: target, resume: true, debounce: false)
            }
            stallRecoveryTask = nil
        }
    }
    private func configureAudioSession() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: .moviePlayback, options: [])
        try? session.setActive(true)
    }

    private func loadCurrent(trackResume: Bool) {
        let item = AVPlayerItem(url: currentURL)
        item.preferredForwardBufferDuration = 2
        player.replaceCurrentItem(with: item)
        player.automaticallyWaitsToMinimizeStalling = true
        poster = currentAssetId.flatMap { thumbService.images[$0] }
        if poster == nil, let assetId = currentAssetId, let client = model.apiClient {
            Task {
                if let image = await thumbService.image(assetId: assetId, client: client) {
                    poster = image
                }
            }
        }

        player.isMuted = muted
        player.volume = volume
        currentTime = 0
        duration = 0
        resumeVisible = false
        resumeChecked = !trackResume
        lastSavedAt = 0
        didReachEnd = false
        usedFallback = false
        subtitleTracks = []
        subtitleCues = []
        subtitleTitle = nil
        if trackResume, startAt > 1 {
            begin(from: startAt)
        } else if trackResume {
            Task { await checkResume() }
        } else {
            begin(from: 0)
        }
        Task { await refreshAccelLabel() }
        Task { await loadSubtitles() }
    }

    @MainActor
    private func checkResume() async {
        guard !resumeChecked else { return }
        resumeChecked = true
        guard let currentAssetId, let client = model.apiClient else {
            begin(from: 0)
            return
        }
        if let saved = try? await client.playbackProgress(assetId: currentAssetId), saved > 2 {
            resumeSeconds = saved
            begin(from: saved)
            resumeToastSeconds = saved
            showResumeToast = true
            Task {
                try? await Task.sleep(nanoseconds: 6_000_000_000)
                showResumeToast = false
            }
        } else {
            begin(from: 0)
        }
    }

    @MainActor
    private func refreshAccelLabel() async {
        guard let currentAssetId, let client = model.apiClient else {
            accelLabel = nil
            return
        }
        guard accelLabel == nil else { return }
        guard let info = try? await client.mediaStreamInfo(assetId: currentAssetId) else {
            accelLabel = nil
            return
        }
        if info.accelerated {
            accelLabel = "115 直连"
        } else if info.provider == "cd2" {
            accelLabel = "CD2 中转"
        } else if info.provider == "alist" {
            accelLabel = "AList 中转"
        } else {
            accelLabel = nil
        }
    }

    private var resolvedIndex: Int? {
        if let currentIndex, playlist.indices.contains(currentIndex) {
            return currentIndex
        }
        guard let currentAssetId else { return nil }
        return playlist.firstIndex(where: { $0.assetId == currentAssetId })
    }

    private var canGoPrevious: Bool {
        guard let index = resolvedIndex else { return false }
        return index > 0
    }

    private var canGoNext: Bool {
        guard let index = resolvedIndex else { return false }
        return index + 1 < playlist.count
    }

    @MainActor
    private func playRelative(_ offset: Int) async {
        guard let index = resolvedIndex else {
            showHUD("播放列表未就绪", seconds: 2)
            return
        }
        let target = index + offset
        guard playlist.indices.contains(target) else { return }
        await playIndex(target)
        showHUD(offset > 0 ? "下一集" : "上一集", seconds: 1.2)
    }

    @MainActor
    private func playIndex(_ index: Int) async {
        guard playlist.indices.contains(index) else { return }
        let item = playlist[index]
        guard let assetId = item.assetId else {
            showHUD("该视频无法播放", seconds: 2)
            return
        }
        guard let source = await model.playbackSource(assetId: assetId, remotePath: item.remotePath) else {
            showHUD("获取播放地址失败", seconds: 2)
            return
        }
        saveProgress()
        currentIndex = index
        currentTitle = item.title
        currentAssetId = assetId
        currentURL = source.url
        currentFallbackURL = source.fallbackURL
        currentSourceLabel = source.label
        accelLabel = source.label
        usedFallback = false
        showPlaylist = false
        loadCurrent(trackResume: false)
    }

    private func toggleAspect() {
        videoGravity = videoGravity == .resizeAspect ? .resizeAspectFill : .resizeAspect
    }

    private func seekToStart() {
        showResumeToast = false
        player.seek(to: .zero)
        currentTime = 0
    }

    private func begin(from seconds: Double) {
        resumeVisible = false
        player.rate = rate
        player.play()
        isPlaying = true
        scheduleHideControls()
        if seconds > 1 {
            Task { @MainActor in
                guard await waitUntilReady() else { return }
                let total = player.currentItem?.duration.seconds ?? 0
                let target = (total.isFinite && total > 5 && seconds > total - 3) ? 0 : seconds
                requestSeek(to: target, resume: true, debounce: false)
                if target == 0 {
                    resumeToastSeconds = 0
                    showResumeToast = false
                }
            }
        }
    }

    private func toggleOrientation() {
        let isLandscape = UIScreen.main.bounds.width > UIScreen.main.bounds.height
        OrientationController.shared.lock(isLandscape ? .portrait : .landscapeRight)
    }

    private func finishScrub() {
        isScrubbing = false
        guard let target = pendingSeek else { return }
        pendingSeek = nil
        currentTime = target
        let shouldPlay = isPlaying
        requestSeek(to: target, resume: shouldPlay)
    }

    private func togglePlay() {
        if player.timeControlStatus == .playing {
            player.pause()
            isPlaying = false
            controlsVisible = true
            controlsTask?.cancel()
        } else {
            player.play()
            isPlaying = true
            scheduleHideControls()
        }
    }

    private func seek(by delta: Double) {
        let target = min(max(0, currentTime + delta), max(0, duration))
        currentTime = target
        let shouldPlay = isPlaying
        requestSeek(to: target, resume: shouldPlay)
        showHUD(delta > 0 ? "+10 秒" : "-10 秒")
    }

    private func setRate(_ value: Float) {
        rate = value
        if isPlaying { player.rate = value }
        showHUD(String(format: "%.2gx", value))
    }

    private func toggleControls() {
        controlsVisible.toggle()
        if controlsVisible {
            scheduleHideControls()
        } else {
            controlsTask?.cancel()
        }
    }

    private func scheduleHideControls() {
        controlsTask?.cancel()
        controlsTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            if !Task.isCancelled, isPlaying {
                controlsVisible = false
            }
        }
    }

    // MARK: - 手势

    private func handleDrag(_ value: DragGesture.Value, size: CGSize) {
        guard !isLocked else { return }
        let moved = max(abs(value.translation.width), abs(value.translation.height))

        // 长按判定：按住不动 0.5 秒才进入 2x，松手立即恢复。
        if dragMode == nil && !isBoosting && pressTask == nil && moved < 6 {
            pressTask = Task { @MainActor in
                try? await Task.sleep(nanoseconds: 500_000_000)
                guard !Task.isCancelled, dragMode == nil else { return }
                isBoosting = true
                player.rate = 2
                showHUD("2x 快进", seconds: 5)
            }
            return
        }

        if isBoosting { return }

        if pressTask != nil, moved >= 6 {
            pressTask?.cancel()
            pressTask = nil
        }

        if dragMode == nil {
            guard moved >= 20 else { return }
            let dx = abs(value.translation.width)
            let dy = abs(value.translation.height)
            if dx > dy * 1.5 {
                dragMode = .seek
                dragBaseTime = currentTime
                seekWasPlaying = isPlaying
                isPlaying = false
                player.pause()
                lastSeekHapticSecond = currentTime
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
            } else if dy > dx * 1.5 {
                if value.startLocation.x < size.width / 2 {
                    dragMode = .brightness
                    dragBaseBrightness = UIScreen.main.brightness
                } else {
                    dragMode = .volume
                    dragBaseVolume = volume
                }
            } else {
                return
            }
        }

        switch dragMode {
        case .seek:
            let range = min(max(duration, 60), scrubRangeSetting)
            let raw = Double(value.translation.width / max(1, size.width)) * range
            var target = dragBaseTime + raw
            let upper = max(0, duration)
            // 越界时给 25% 阻尼，松手回弹，避免拖出边界没有“实感”。
            if target < 0 { target *= 0.25 }
            if target > upper { target = upper + (target - upper) * 0.25 }
            seekPreview = target
            if abs(target - lastSeekHapticSecond) >= 8 {
                lastSeekHapticSecond = target
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
            }
        case .brightness:
            let ratio = -Double(value.translation.height / max(1, size.height))
            let next = min(max(0.05, dragBaseBrightness + CGFloat(ratio) * 0.9), 1)
            UIScreen.main.brightness = next
            showHUD("亮度 \(Int(next * 100))%", seconds: 6)
        case .volume:
            let ratio = -Double(value.translation.height / max(1, size.height))
            let next = min(max(0, dragBaseVolume + Float(ratio) * 0.9), 1)
            volume = next
            player.volume = next
            showHUD("音量 \(Int(next * 100))%", seconds: 6)
        case .none:
            break
        }
    }

    private func endDrag(_ value: DragGesture.Value, size: CGSize) {
        guard !isLocked else {
            pressTask?.cancel()
            pressTask = nil
            return
        }
        pressTask?.cancel()
        pressTask = nil
        if isBoosting {
            isBoosting = false
            player.rate = rate
            hideHUD()
            dragMode = nil
            seekPreview = nil
            return
        }
        if dragMode == .seek, let target = seekPreview {
            let clamped = min(max(0, target), max(0, duration))
            player.seek(to: CMTime(seconds: clamped, preferredTimescale: 600))
            currentTime = clamped
            saveProgress()
            if seekWasPlaying {
                player.play()
                isPlaying = true
            }
        }
        seekWasPlaying = false
        dragMode = nil
        seekPreview = nil
        hideHUD()
    }

    // MARK: - HUD / 进度

    private func showHUD(_ text: String, seconds: Double = 1.6) {
        hudText = text
        hudTask?.cancel()
        hudTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            if !Task.isCancelled { hudText = nil }
        }
    }

    private func hideHUD() {
        hudTask?.cancel()
        hudText = nil
    }

    private func addTimeObserver() {
        timeObserver = player.addPeriodicTimeObserver(
            forInterval: CMTime(seconds: 0.5, preferredTimescale: 600),
            queue: .main
        ) { time in
            if !isScrubbing, pendingSeek == nil {
                currentTime = time.seconds.isFinite ? time.seconds : 0
            }
            if duration > 1, currentTime >= duration - 0.6, !didReachEnd {
                didReachEnd = true
                handlePlaybackEnd()
            }
            if currentTime < 1 {
                didReachEnd = false
            }
            if let item = player.currentItem {
                let seconds = item.duration.seconds
                duration = seconds.isFinite ? seconds : 0
            }
            if player.currentItem?.status == .failed, !usedFallback {
                usedFallback = true
                let target = currentTime
                let wasPlaying = isPlaying
                Task { @MainActor in
                    var fallback = currentFallbackURL
                    if fallback == nil, let assetId = currentAssetId {
                        fallback = await model.fallbackMediaURL(assetId: assetId)
                    }
                    guard let fallback else {
                        usedFallback = false
                        return
                    }
                    player.replaceCurrentItem(with: AVPlayerItem(url: fallback))
                    accelLabel = "服务器中转"
                    poster = nil
                    if await waitUntilReady() {
                        requestSeek(to: target, resume: wasPlaying, debounce: false)
                    }
                }
            }
            isBuffering = player.timeControlStatus == .waitingToPlayAtSpecifiedRate
            if isBuffering, isPlaying, player.currentItem?.status == .readyToPlay {
                recoverFromStall()
            } else {
                stallRecoveryTask?.cancel()
                stallRecoveryTask = nil
            }
            if currentTime - lastSavedAt >= 3 {
                lastSavedAt = currentTime
                saveProgress()
            }
        }
    }

    private func saveProgress() {
        guard let currentAssetId, let client = model.apiClient else { return }
        let seconds = player.currentTime().seconds
        guard seconds.isFinite, seconds > 0 else { return }
        Task { try? await client.setPlaybackProgress(assetId: currentAssetId, seconds: seconds) }
    }

    private func teardown() {
        if let timeObserver {
            player.removeTimeObserver(timeObserver)
        }
        timeObserver = nil
        saveProgress()
        player.pause()
        UIScreen.main.brightness = baseBrightness
        controlsTask?.cancel()
        hudTask?.cancel()
        pressTask?.cancel()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
}

struct PlayerLayerContainer: UIViewRepresentable {
    let player: AVPlayer
    var gravity: AVLayerVideoGravity = .resizeAspect

    func makeUIView(context: Context) -> VideoLayerUIView {
        let view = VideoLayerUIView()
        view.player = player
        view.videoGravity = gravity
        return view
    }

    func updateUIView(_ uiView: VideoLayerUIView, context: Context) {
        uiView.player = player
        uiView.videoGravity = gravity
    }
}

final class VideoLayerUIView: UIView {
    override class var layerClass: AnyClass { AVPlayerLayer.self }

    private var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }

    var player: AVPlayer? {
        get { playerLayer.player }
        set { playerLayer.player = newValue }
    }

    var videoGravity: AVLayerVideoGravity {
        get { playerLayer.videoGravity }
        set { playerLayer.videoGravity = newValue }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        playerLayer.videoGravity = .resizeAspect
        backgroundColor = .black
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }
}

/// 细轨道进度条：小圆点、拖动放大，替代系统 Slider 的大节点。
private struct PlayerProgressSlider: View {
    let value: Double
    let duration: Double
    let onScrubStart: () -> Void
    let onScrub: (Double) -> Void
    let onScrubEnd: (Double) -> Void

    @State private var dragging = false

    var body: some View {
        GeometryReader { geo in
            let progress = min(max(value / max(duration, 1), 0), 1)
            ZStack(alignment: .leading) {
                Capsule().fill(Color.white.opacity(0.25)).frame(height: 3)
                Capsule().fill(Color.white).frame(width: geo.size.width * progress, height: 3)
                Circle()
                    .fill(Color.white)
                    .frame(width: dragging ? 14 : 9, height: dragging ? 14 : 9)
                    .offset(x: geo.size.width * progress - (dragging ? 7 : 4.5))
            }
            .frame(maxHeight: .infinity, alignment: .center)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { drag in
                        if !dragging {
                            dragging = true
                            onScrubStart()
                        }
                        let fraction = min(max(drag.location.x / geo.size.width, 0), 1)
                        onScrub(fraction * duration)
                    }
                    .onEnded { drag in
                        let fraction = min(max(drag.location.x / geo.size.width, 0), 1)
                        dragging = false
                        onScrubEnd(fraction * duration)
                    }
            )
        }
        .frame(height: 26)
    }
}