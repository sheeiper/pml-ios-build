import SwiftUI
import UIKit
import AVFoundation

#if canImport(KSPlayer)
import KSPlayer
#if canImport(KSPlayerUI)
import KSPlayerUI
#endif
#endif

/// 判断文件扩展名是否属于 AVPlayer 原生不支持、需要默认使用兼容内核（KSPlayer / FFmpeg + Metal）的格式。
enum PlaybackEngineSelector {
    static let compatibilityPreferredExtensions: Set<String> = [
        "mkv", "wmv", "wma", "avi", "flv", "rmvb", "rm",
        "ts", "m2ts", "mts", "webm", "ogv", "asf", "vob",
        "f4v", "mpeg", "mpg", "divx", "3gp", "3g2", "mxf", "opus", "ogg", "flac"
    ]

    static var isCompatibilityEngineAvailable: Bool {
        #if canImport(KSPlayer)
        return true
        #else
        return false
        #endif
    }

    // 兼容历史命名
    static var isVLCAvailable: Bool {
        isCompatibilityEngineAvailable
    }

    static func shouldPreferCompatibilityEngine(url: URL, title: String, remotePath: String?) -> Bool {
        guard isCompatibilityEngineAvailable else { return false }
        let candidates = [
            url.pathExtension,
            (title as NSString).pathExtension,
            remotePath.map { ($0 as NSString).pathExtension } ?? ""
        ]
        for raw in candidates {
            let ext = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if !ext.isEmpty && compatibilityPreferredExtensions.contains(ext) {
                return true
            }
        }
        return false
    }

    // 兼容历史命名
    static func shouldPreferVLC(url: URL, title: String, remotePath: String?) -> Bool {
        shouldPreferCompatibilityEngine(url: url, title: title, remotePath: remotePath)
    }
}

/// 封装 KSPlayer（Metal + FFmpeg/VideoToolbox 硬解），用于 MKV/WMV/WMA/AVI/FLV 全格式直链播放，
/// 具备原生级画中画 (PiP) 支持，并彻底修复部分 FC2 非标 B 帧 / VFR 视频在原生 AVPlayer 下的黑屏与画面抽搐问题。
final class KSPlaybackEngine: NSObject, ObservableObject {
    @Published var currentTime: Double = 0
    @Published var duration: Double = 0
    @Published var isPlaying: Bool = false
    @Published var isBuffering: Bool = false
    @Published var hasFailed: Bool = false
    @Published var didReachEnd: Bool = false

    var onTimeUpdate: ((Double, Double) -> Void)?
    var onPlayingChanged: ((Bool) -> Void)?
    var onBufferingChanged: ((Bool) -> Void)?
    var onFailed: (() -> Void)?
    var onEnded: (() -> Void)?

    #if canImport(KSPlayer)
    private var playerView: IOSVideoPlayerView?
    private var pendingStartTime: Double = 0
    private var didApplyStartTime: Bool = false
    private var desiredRate: Float = 1.0
    private var desiredVolume: Float = 1.0
    private var desiredMuted: Bool = false
    private var isStopped: Bool = true
    #endif

    override init() {
        super.init()
    }

    #if canImport(KSPlayer)
    func makePlayerView() -> IOSVideoPlayerView {
        if let existing = playerView {
            return existing
        }
        let view = IOSVideoPlayerView()
        // 隐藏 KSPlayer 默认自带的覆盖控件，统一使用项目自研的手势、进度与工具栏
        view.toolBar.isHidden = true
        view.navigationBar.isHidden = true
        view.loadingView.isHidden = true
        view.delegate = self
        view.playTimeDidChange = { [weak self] current, total in
            DispatchQueue.main.async {
                guard let self = self, !self.isStopped else { return }
                self.currentTime = current
                if total > 0 {
                    self.duration = total
                }
                if self.isBuffering && current > 0 {
                    self.isBuffering = false
                    self.onBufferingChanged?(false)
                }
                self.onTimeUpdate?(current, self.duration)
            }
        }
        self.playerView = view
        return view
    }
    #endif

    func attachDrawable(_ view: UIView) {
        // 兼容原 VLC 接口形态
    }

    func load(url: URL, startAt: Double = 0, rate: Float = 1.0, volume: Float = 1.0, muted: Bool = false) {
        #if canImport(KSPlayer)
        stop()
        hasFailed = false
        didReachEnd = false
        isBuffering = true
        isStopped = false
        pendingStartTime = max(0, startAt)
        didApplyStartTime = startAt <= 1.0
        desiredRate = rate
        desiredVolume = volume
        desiredMuted = muted
        currentTime = max(0, startAt)
        duration = 0

        let options = KSOptions()
        options.isAccurateSeek = true
        options.isHardwareDecode = true
        // 针对特殊容器格式使用 KSMEPlayer (FFmpeg/Metal) 解码
        KSOptions.firstPlayerType = KSMEPlayer.self

        let view = makePlayerView()
        view.set(url: url, options: options)
        applyAudioAndRate()
        #else
        hasFailed = true
        onFailed?()
        #endif
    }

    func play() {
        #if canImport(KSPlayer)
        guard let view = playerView else { return }
        view.player?.play()
        applyAudioAndRate()
        #endif
    }

    func pause() {
        #if canImport(KSPlayer)
        playerView?.player?.pause()
        #endif
    }

    func seek(to seconds: Double) {
        #if canImport(KSPlayer)
        guard let view = playerView else { return }
        let clamped = max(0, seconds)
        currentTime = clamped
        view.seek(time: clamped)
        #endif
    }

    func setRate(_ rate: Float) {
        #if canImport(KSPlayer)
        desiredRate = rate
        playerView?.player?.playbackRate = rate
        #endif
    }

    func setVolume(_ volume: Float) {
        #if canImport(KSPlayer)
        desiredVolume = min(max(0, volume), 1)
        playerView?.player?.playbackVolume = desiredVolume
        #endif
    }

    func setMuted(_ muted: Bool) {
        #if canImport(KSPlayer)
        desiredMuted = muted
        playerView?.player?.isMuted = muted
        #endif
    }

    func stop() {
        #if canImport(KSPlayer)
        isStopped = true
        if let view = playerView {
            view.player?.pause()
            view.resetPlayer()
        }
        playerView = nil
        isPlaying = false
        isBuffering = false
        #endif
    }

    #if canImport(KSPlayer)
    private func applyAudioAndRate() {
        guard let player = playerView?.player else { return }
        player.isMuted = desiredMuted
        player.playbackVolume = desiredVolume
        if abs(desiredRate - 1.0) > 0.01 {
            player.playbackRate = desiredRate
        }
    }
    #endif
}

#if canImport(KSPlayer)
extension KSPlaybackEngine: PlayerControllerDelegate {
    func playerController(state: KSPlayerState) {
        DispatchQueue.main.async { [weak self] in
            guard let self = self, !self.isStopped else { return }
            switch state {
            case .buffering, .preparing:
                self.isBuffering = true
                self.onBufferingChanged?(true)
            case .readyToPlay:
                if !self.didApplyStartTime && self.pendingStartTime > 1.0 {
                    self.didApplyStartTime = true
                    self.playerView?.seek(time: self.pendingStartTime)
                }
                self.applyAudioAndRate()
                self.isBuffering = false
                self.isPlaying = true
                self.onPlayingChanged?(true)
                self.onBufferingChanged?(false)
            case .bufferFinished:
                self.isBuffering = false
                self.isPlaying = true
                self.onPlayingChanged?(true)
                self.onBufferingChanged?(false)
            case .paused:
                self.isPlaying = false
                self.isBuffering = false
                self.onPlayingChanged?(false)
                self.onBufferingChanged?(false)
            case .playedToTheEnd:
                self.isPlaying = false
                self.isBuffering = false
                self.didReachEnd = true
                self.onPlayingChanged?(false)
                self.onEnded?()
            case .error:
                self.isPlaying = false
                self.isBuffering = false
                self.hasFailed = true
                self.onPlayingChanged?(false)
                self.onFailed?()
            default:
                break
            }
        }
    }

    func playerController(currentTime: TimeInterval, totalTime: TimeInterval) {
        DispatchQueue.main.async { [weak self] in
            guard let self = self, !self.isStopped else { return }
            self.currentTime = currentTime
            if totalTime > 0 {
                self.duration = totalTime
            }
            self.onTimeUpdate?(currentTime, self.duration)
        }
    }

    func playerController(finish error: Error?) {
        DispatchQueue.main.async { [weak self] in
            guard let self = self, !self.isStopped else { return }
            if error != nil {
                self.hasFailed = true
                self.onFailed?()
            }
        }
    }

    func playerController(maskShow: Bool) {}
    func playerController(action: PlayerButtonType) {}
    func playerController(bufferedCount: Int, consumeTime: TimeInterval) {}
}
#endif

/// SwiftUI 容器视图，用于承载 KSPlayer 视频渲染层。
struct KSPlayerContainer: UIViewRepresentable {
    let engine: KSPlaybackEngine

    func makeUIView(context: Context) -> UIView {
        #if canImport(KSPlayer)
        let playerView = engine.makePlayerView()
        playerView.backgroundColor = .black
        playerView.clipsToBounds = true
        return playerView
        #else
        let view = UIView()
        view.backgroundColor = .black
        view.clipsToBounds = true
        return view
        #endif
    }

    func updateUIView(_ uiView: UIView, context: Context) {}
}

// 保持历史兼容别名
typealias VLCPlaybackEngine = KSPlaybackEngine
typealias VLCPlayerContainer = KSPlayerContainer
