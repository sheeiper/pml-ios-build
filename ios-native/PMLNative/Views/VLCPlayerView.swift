import SwiftUI
import UIKit

#if canImport(VLCKitSPM)
import VLCKitSPM
#endif

/// 判断文件扩展名是否属于 AVPlayer 原生不支持、需要默认使用 VLC/FFmpeg 兼容内核的格式。
enum PlaybackEngineSelector {
    static let vlcPreferredExtensions: Set<String> = [
        "mkv", "wmv", "wma", "avi", "flv", "rmvb", "rm",
        "ts", "m2ts", "mts", "webm", "ogv", "asf", "vob",
        "f4v", "mpeg", "mpg", "divx", "3gp", "3g2", "mxf", "opus", "ogg", "flac"
    ]

    static var isVLCAvailable: Bool {
        #if canImport(VLCKitSPM)
        return true
        #else
        return false
        #endif
    }

    static func shouldPreferVLC(url: URL, title: String, remotePath: String?) -> Bool {
        guard isVLCAvailable else { return false }
        let candidates = [
            url.pathExtension,
            (title as NSString).pathExtension,
            remotePath.map { ($0 as NSString).pathExtension } ?? ""
        ]
        for raw in candidates {
            let ext = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if !ext.isEmpty && vlcPreferredExtensions.contains(ext) {
                return true
            }
        }
        return false
    }
}

/// 封装 MobileVLCKit 3.6.0（FFmpeg + VideoToolbox 硬解），用于 MKV/WMV/WMA/AVI/FLV 全格式直链播放
/// 以及修复部分 FC2 非标 B 帧 / VFR 视频在原生 AVPlayer 下的黑屏与画面抽搐问题。
final class VLCPlaybackEngine: NSObject, ObservableObject {
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

    #if canImport(VLCKitSPM)
    private var mediaPlayer: VLCMediaPlayer?
    private weak var drawableView: UIView?
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

    func attachDrawable(_ view: UIView) {
        #if canImport(VLCKitSPM)
        drawableView = view
        mediaPlayer?.drawable = view
        #endif
    }

    func load(url: URL, startAt: Double = 0, rate: Float = 1.0, volume: Float = 1.0, muted: Bool = false) {
        #if canImport(VLCKitSPM)
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

        let options: [String] = [
            "--avcodec-hw=any",
            "--network-caching=2500",
            "--file-caching=1500",
            "--live-caching=2000",
            "--clock-jitter=0",
            "--drop-late-frames",
            "--skip-frames",
            "--http-reconnect"
        ]
        let player = VLCMediaPlayer(options: options)
        player.delegate = self
        if let drawableView {
            player.drawable = drawableView
        }

        let media = VLCMedia(url: url)
        media.addOptions([
            "network-caching": 2500,
            "file-caching": 1500,
            "avcodec-hw": "any",
            "clock-jitter": 0,
            "http-reconnect": true
        ])
        player.media = media
        self.mediaPlayer = player

        player.play()
        applyAudioState()
        if abs(desiredRate - 1.0) > 0.01 {
            player.rate = desiredRate
        }
        #else
        hasFailed = true
        onFailed?()
        #endif
    }

    func play() {
        #if canImport(VLCKitSPM)
        guard let player = mediaPlayer else { return }
        player.play()
        if abs(desiredRate - 1.0) > 0.01 {
            player.rate = desiredRate
        }
        #endif
    }

    func pause() {
        #if canImport(VLCKitSPM)
        mediaPlayer?.pause()
        #endif
    }

    func seek(to seconds: Double) {
        #if canImport(VLCKitSPM)
        guard let player = mediaPlayer else { return }
        let clamped = max(0, seconds)
        let ms = Int32(clamped * 1000.0)
        player.time = VLCTime(int: ms)
        currentTime = clamped
        #endif
    }

    func setRate(_ rate: Float) {
        #if canImport(VLCKitSPM)
        desiredRate = rate
        mediaPlayer?.rate = rate
        #endif
    }

    func setVolume(_ volume: Float) {
        #if canImport(VLCKitSPM)
        desiredVolume = min(max(0, volume), 1)
        applyAudioState()
        #endif
    }

    func setMuted(_ muted: Bool) {
        #if canImport(VLCKitSPM)
        desiredMuted = muted
        applyAudioState()
        #endif
    }

    func stop() {
        #if canImport(VLCKitSPM)
        isStopped = true
        if let player = mediaPlayer {
            player.delegate = nil
            player.stop()
            player.drawable = nil
        }
        mediaPlayer = nil
        isPlaying = false
        isBuffering = false
        #endif
    }

    #if canImport(VLCKitSPM)
    private func applyAudioState() {
        guard let audio = mediaPlayer?.audio else { return }
        audio.isMuted = desiredMuted
        audio.volume = Int32(round(desiredVolume * 100.0))
    }
    #endif
}

#if canImport(VLCKitSPM)
extension VLCPlaybackEngine: VLCMediaPlayerDelegate {
    func mediaPlayerTimeChanged(_ aNotification: Notification) {
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.isStopped, let player = self.mediaPlayer else { return }
            let timeMs = Double(player.time.intValue)
            let durMs = Double(player.media?.length.intValue ?? 0)
            let secs = max(0, timeMs / 1000.0)
            let total = max(0, durMs / 1000.0)

            if !self.didApplyStartTime && self.pendingStartTime > 1.0 && total > self.pendingStartTime {
                self.didApplyStartTime = true
                player.time = VLCTime(int: Int32(self.pendingStartTime * 1000.0))
                self.applyAudioState()
                if abs(self.desiredRate - 1.0) > 0.01 {
                    player.rate = self.desiredRate
                }
                return
            }

            self.currentTime = secs
            if total > 0 {
                self.duration = total
            }
            if self.isBuffering && player.isPlaying {
                self.isBuffering = false
                self.onBufferingChanged?(false)
            }
            self.onTimeUpdate?(secs, self.duration)
        }
    }

    func mediaPlayerStateChanged(_ aNotification: Notification) {
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.isStopped, let player = self.mediaPlayer else { return }
            switch player.state {
            case .opening, .buffering:
                self.isBuffering = true
                self.onBufferingChanged?(true)
            case .playing:
                self.isPlaying = true
                self.isBuffering = false
                self.applyAudioState()
                if abs(self.desiredRate - 1.0) > 0.01 {
                    player.rate = self.desiredRate
                }
                self.onPlayingChanged?(true)
                self.onBufferingChanged?(false)
            case .paused:
                self.isPlaying = false
                self.isBuffering = false
                self.onPlayingChanged?(false)
                self.onBufferingChanged?(false)
            case .ended:
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
}
#endif

/// SwiftUI 容器视图，用于承载 VLC 视频渲染层。
struct VLCPlayerContainer: UIViewRepresentable {
    let engine: VLCPlaybackEngine

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.backgroundColor = .black
        view.clipsToBounds = true
        engine.attachDrawable(view)
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        engine.attachDrawable(uiView)
    }
}
