import SwiftUI
import AVKit
import AVFoundation

enum OnlinePreviewMode: String, Identifiable {
    case trailer
    case stream

    var id: String { rawValue }

    var badgeText: String {
        self == .trailer ? "FANZA 预告片" : "HLS 在线预览"
    }
}

struct OnlinePreviewPlayerSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let mode: OnlinePreviewMode
    let number: String
    let title: String
    var coverUrl: String? = nil

    @State private var loading = true
    @State private var errorMessage: String?
    @State private var trailer: MovieTrailerResult?
    @State private var stream: OnlineStreamResult?
    @State private var selectedQualityIdx = 0
    @State private var selectedLineIdx = 0

    @State private var player: AVPlayer?
    @State private var timeObserver: Any?
    @State private var currentTime: Double = 0
    @State private var duration: Double = 0

    // Subtitle mounting + timing offset
    @State private var showSubtitlePicker = false
    @State private var subSearching = false
    @State private var subDownloadingId: String?
    @State private var subCandidates: [SubtitleSearchCandidate] = []
    @State private var subError: String?
    @State private var subtitleCues: [SubtitleCue] = []
    @State private var activeSubName = ""
    @State private var subtitleOffset: Double = 0

    private var activeTrailerItem: TrailerQualityItem? {
        guard let items = trailer?.items, !items.isEmpty else { return nil }
        return items[min(selectedQualityIdx, items.count - 1)]
    }

    private var activeStreamSource: OnlineStreamSource? {
        guard let sources = stream?.sources, !sources.isEmpty else { return nil }
        return sources[min(selectedLineIdx, sources.count - 1)]
    }

    private var currentCueText: String? {
        guard !subtitleCues.isEmpty else { return nil }
        return SubtitleParser.cue(at: currentTime + subtitleOffset, in: subtitleCues)?.text
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 12) {
                playerSurface
                controlsBar
                if let err = errorMessage {
                    VStack(alignment: .leading, spacing: 8) {
                        Label(err, systemImage: "exclamationmark.triangle")
                            .font(.caption)
                            .foregroundStyle(.orange)
                        fallbackLinkView
                    }
                    .padding(.horizontal, 14)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 10)
            .background(Color.black.ignoresSafeArea())
            .navigationTitle("\(number) · \(mode.badgeText)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("关闭") { dismiss() }
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button {
                        showSubtitlePicker = true
                        if subCandidates.isEmpty && !subSearching {
                            Task { await searchSubtitles() }
                        }
                    } label: {
                        Image(systemName: subtitleCues.isEmpty ? "captions.bubble" : "captions.bubble.fill")
                    }
                    Button {
                        Task { await loadMedia(forceRefresh: true) }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                }
            }
            .sheet(isPresented: $showSubtitlePicker) {
                subtitlePickerSheet
            }
            .task {
                await loadMedia(forceRefresh: false)
            }
            .onDisappear {
                teardownPlayer()
            }
        }
        .preferredColorScheme(.dark)
    }

    @ViewBuilder
    private var playerSurface: some View {
        ZStack(alignment: .bottom) {
            if loading {
                RoundedRectangle(cornerRadius: 12)
                    .fill(Color.white.opacity(0.06))
                    .aspectRatio(16 / 9, contentMode: .fit)
                    .overlay {
                        VStack(spacing: 8) {
                            ProgressView()
                            Text(mode == .trailer ? "正在解析 FANZA 多码率预告片…" : "正在检索在线 HLS片源…")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
            } else if let player {
                VideoPlayer(player: player)
                    .aspectRatio(16 / 9, contentMode: .fit)
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
            } else {
                RoundedRectangle(cornerRadius: 12)
                    .fill(Color.white.opacity(0.05))
                    .aspectRatio(16 / 9, contentMode: .fit)
                    .overlay {
                        Image(systemName: "film")
                            .font(.largeTitle)
                            .foregroundStyle(.secondary)
                    }
            }

            if let text = currentCueText, !text.isEmpty {
                Text(text)
                    .font(.subheadline.weight(.semibold))
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.white)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(.black.opacity(0.76), in: RoundedRectangle(cornerRadius: 6))
                    .padding(.horizontal, 14)
                    .padding(.bottom, 18)
                    .allowsHitTesting(false)
            }
        }
        .padding(.horizontal, 12)
    }

    @ViewBuilder
    private var controlsBar: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.subheadline.weight(.semibold))
                .lineLimit(2)
                .padding(.horizontal, 14)

            if mode == .trailer, let items = trailer?.items, !items.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        Text("清晰度").font(.caption2).foregroundStyle(.secondary)
                        ForEach(Array(items.enumerated()), id: \.offset) { idx, q in
                            Button(q.label) {
                                switchQuality(idx)
                            }
                            .font(.caption.bold())
                            .buttonStyle(.bordered)
                            .tint(idx == selectedQualityIdx ? .purple : .secondary)
                            .controlSize(.small)
                        }
                    }
                    .padding(.horizontal, 14)
                }
            }

            if mode == .stream, let stream, !stream.sources.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        Text("线路 (\(stream.via))").font(.caption2).foregroundStyle(.secondary)
                        ForEach(Array(stream.sources.enumerated()), id: \.offset) { idx, src in
                            Button(src.name) {
                                selectedLineIdx = idx
                                attachStreamSource()
                            }
                            .font(.caption.bold())
                            .buttonStyle(.bordered)
                            .tint(idx == selectedLineIdx ? .green : .secondary)
                            .controlSize(.small)
                        }
                    }
                    .padding(.horizontal, 14)
                }
            }

            if !subtitleCues.isEmpty {
                HStack(spacing: 8) {
                    Image(systemName: "captions.bubble.fill")
                        .font(.caption)
                        .foregroundStyle(.purple)
                    Text(activeSubName)
                        .font(.caption2)
                        .lineLimit(1)
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("-0.5s") { subtitleOffset -= 0.5 }
                        .font(.caption2.bold())
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                    Button(String(format: "%.1fs", subtitleOffset)) { subtitleOffset = 0 }
                        .font(.caption2.monospacedDigit())
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                    Button("+0.5s") { subtitleOffset += 0.5 }
                        .font(.caption2.bold())
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                }
                .padding(.horizontal, 14)
            }
        }
    }

    @ViewBuilder
    private var fallbackLinkView: some View {
        if mode == .trailer, let raw = trailer?.fallbackUrl, let url = URL(string: raw), !raw.isEmpty {
            Link("在浏览器打开备用预告片搜索", destination: url)
                .font(.caption.weight(.semibold))
        } else if mode == .stream, let raw = stream?.searchUrl, let url = URL(string: raw), !raw.isEmpty {
            Link("在浏览器打开在线搜索页", destination: url)
                .font(.caption.weight(.semibold))
        }
    }

    private var subtitlePickerSheet: some View {
        NavigationStack {
            List {
                if let subError {
                    Text(subError).font(.caption).foregroundStyle(.red)
                }
                if subSearching {
                    HStack(spacing: 8) {
                        ProgressView()
                        Text("正在从迅雷与 SubtitleCat 检索中文字幕…").font(.caption)
                    }
                } else if subCandidates.isEmpty {
                    Text("未找到可用的简/繁体字幕候选。").font(.caption).foregroundStyle(.secondary)
                } else {
                    ForEach(subCandidates) { cand in
                        HStack(spacing: 8) {
                            VStack(alignment: .leading, spacing: 3) {
                                HStack(spacing: 5) {
                                    Text(cand.langLabel.isEmpty ? cand.lang : cand.langLabel)
                                        .font(.system(size: 10, weight: .bold))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1.5)
                                        .background((cand.lang == "zh-CN" ? Color.blue : Color.orange).opacity(0.2), in: Capsule())
                                    if !cand.source.isEmpty {
                                        Text(cand.source == "xunlei" ? "迅雷" : cand.source)
                                            .font(.system(size: 10, weight: .bold))
                                            .padding(.horizontal, 5)
                                            .padding(.vertical, 1.5)
                                            .background(Color.cyan.opacity(0.2), in: Capsule())
                                    }
                                    if cand.durationMatched == true {
                                        Text("时长匹配")
                                            .font(.system(size: 10, weight: .bold))
                                            .padding(.horizontal, 5)
                                            .padding(.vertical, 1.5)
                                            .background(Color.green.opacity(0.2), in: Capsule())
                                            .foregroundStyle(.green)
                                    }
                                }
                                Text(cand.title)
                                    .font(.caption)
                                    .lineLimit(2)
                            }
                            Spacer()
                            Button {
                                Task { await mountSubtitle(cand) }
                            } label: {
                                if subDownloadingId == cand.id {
                                    ProgressView().scaleEffect(0.75)
                                } else {
                                    Text("挂载").font(.caption2.bold())
                                }
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.mini)
                            .disabled(subDownloadingId != nil)
                        }
                    }
                }
            }
            .navigationTitle("挂载外挂字幕")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("关闭") { showSubtitlePicker = false }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("刷新") { Task { await searchSubtitles() } }
                        .disabled(subSearching)
                }
            }
        }
    }

    private func loadMedia(forceRefresh: Bool) async {
        guard let client = model.apiClient else {
            errorMessage = "未连接到服务器"
            loading = false
            return
        }
        loading = true
        errorMessage = nil
        teardownPlayer()
        do {
            if mode == .trailer {
                let res = try await client.getMovieTrailer(number: number, coverUrl: coverUrl, forceRefresh: forceRefresh)
                trailer = res
                selectedQualityIdx = 0
                if res.items.isEmpty {
                    errorMessage = "未解析到 FANZA 直链试看视频，可使用下方备用入口。"
                } else {
                    attachTrailerSource()
                }
            } else {
                let res = try await client.getOnlineMovieStream(number: number, forceRefresh: forceRefresh)
                stream = res
                selectedLineIdx = 0
                if res.sources.isEmpty {
                    errorMessage = "暂未找到可用在线片源。"
                } else {
                    attachStreamSource()
                }
            }
        } catch {
            errorMessage = error.localizedDescription
        }
        loading = false
    }

    private func attachTrailerSource() {
        guard let item = activeTrailerItem, let url = URL(string: item.url) else { return }
        startPlayer(url: url, headers: nil, startAt: 0)
    }

    private func switchQuality(_ idx: Int) {
        let saved = currentTime
        selectedQualityIdx = idx
        guard let item = activeTrailerItem, let url = URL(string: item.url) else { return }
        startPlayer(url: url, headers: nil, startAt: saved)
    }

    private func attachStreamSource() {
        guard let client = model.apiClient,
              let src = activeStreamSource,
              let url = client.onlinePreviewM3U8URL(sourceURL: src.url) else {
            errorMessage = "无法构造在线代理流地址"
            return
        }
        startPlayer(url: url, headers: client.basicAuthHeaders, startAt: 0)
    }

    private func startPlayer(url: URL, headers: [String: String]?, startAt: Double) {
        teardownPlayer()
        let asset: AVURLAsset
        if let headers, !headers.isEmpty {
            asset = AVURLAsset(url: url, options: ["AVURLAssetHTTPHeaderFieldsKey": headers])
        } else {
            asset = AVURLAsset(url: url)
        }
        let item = AVPlayerItem(asset: asset)
        let avPlayer = AVPlayer(playerItem: item)
        avPlayer.volume = 0.6
        if startAt > 1 {
            avPlayer.seek(to: CMTime(seconds: startAt, preferredTimescale: 600))
        }
        let interval = CMTime(seconds: 0.25, preferredTimescale: 600)
        timeObserver = avPlayer.addPeriodicTimeObserver(forInterval: interval, queue: .main) { time in
            let sec = CMTimeGetSeconds(time)
            if sec.isFinite { currentTime = sec }
            if let dur = avPlayer.currentItem?.duration, CMTimeGetSeconds(dur).isFinite {
                duration = CMTimeGetSeconds(dur)
            }
        }
        player = avPlayer
        avPlayer.play()
    }

    private func teardownPlayer() {
        if let obs = timeObserver, let p = player {
            p.removeTimeObserver(obs)
        }
        timeObserver = nil
        player?.pause()
        player = nil
    }

    private func searchSubtitles() async {
        guard let client = model.apiClient else { return }
        subSearching = true
        subError = nil
        do {
            let expected = duration > 30 ? Int(duration.rounded()) : nil
            subCandidates = try await client.searchMovieSubtitles(number: number, expectedDurationSec: expected)
        } catch {
            subError = error.localizedDescription
        }
        subSearching = false
    }

    private func mountSubtitle(_ cand: SubtitleSearchCandidate) async {
        guard let client = model.apiClient else { return }
        subDownloadingId = cand.id
        subError = nil
        do {
            let saved = try await client.downloadMovieSubtitle(
                number: number,
                name: cand.title,
                lang: cand.lang,
                downloadUrl: cand.downloadUrl,
                source: cand.source
            )
            let text = try await client.fetchMovieSubtitleText(id: saved.id)
            subtitleCues = SubtitleParser.parse(text)
            activeSubName = cand.title
            subtitleOffset = 0
            showSubtitlePicker = false
        } catch {
            subError = error.localizedDescription
        }
        subDownloadingId = nil
    }
}
