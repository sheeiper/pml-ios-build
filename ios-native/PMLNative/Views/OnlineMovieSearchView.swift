import SwiftUI
import UIKit

/// 影视库独立「搜番号」Tab 页：全网检索番号 + 最近搜索历史 + 点击进入完整详情页（含剧照与磁力）
struct OnlineMovieSearchTabView: View {
    @EnvironmentObject private var model: AppModel
    @Binding var presetQuery: String
    var onPlayAsset: ((String, String) -> Void)? = nil
    var onFilterActor: ((String) -> Void)? = nil

    @State private var query = ""
    @State private var history: [String] = []
    @State private var results: [PerformerRecentRelease] = []
    @State private var loading = false
    @State private var searchedOnce = false
    @State private var errorMessage: String?
    @State private var selectedRelease: PerformerRecentRelease?

    private let historyKey = "pml.onlineMovieSearchHistory"

    var body: some View {
        VStack(spacing: 0) {
            searchBar
            if !history.isEmpty {
                historyStrip
            }
            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14)
                    .padding(.bottom, 6)
            }

            if loading {
                ProgressView("正在全网检索番号…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if results.isEmpty {
                ContentUnavailableView(
                    searchedOnce ? "未找到线上作品" : "输入番号全网检索",
                    systemImage: searchedOnce ? "magnifyingglass" : "globe",
                    description: Text(searchedOnce ? "请检查番号拼写，或点击刷新按钮重试。" : "点击结果卡片可查看高清封面、演员、片厂、剧照与可用磁力链接。")
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                        ForEach(results) { item in
                            Button {
                                selectedRelease = item
                            } label: {
                                resultCard(item)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(14)
                }
            }
        }
        .onAppear {
            loadHistory()
            consumePresetIfNeeded()
        }
        .onChange(of: presetQuery) { _, _ in
            consumePresetIfNeeded()
        }
        .sheet(item: $selectedRelease) { release in
            OnlineMovieDetailSheet(
                release: release,
                onPlayAsset: { assetId, title in
                    selectedRelease = nil
                    onPlayAsset?(assetId, title)
                },
                onFilterActor: { actorName in
                    selectedRelease = nil
                    onFilterActor?(actorName)
                }
            )
            .environmentObject(model)
        }
    }

    private var searchBar: some View {
        HStack(spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                    .font(.subheadline)
                TextField("输入番号，例如 SSIS-123 / FC2-4833053", text: $query)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
                    .onSubmit { Task { await runSearch(forceRefresh: false) } }
                if !query.isEmpty {
                    Button {
                        query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.secondary)
                            .font(.subheadline)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            Button {
                Task { await runSearch(forceRefresh: false) }
            } label: {
                Text("搜索")
                    .font(.subheadline.weight(.semibold))
            }
            .buttonStyle(.borderedProminent)
            .tint(.purple)
            .disabled(loading || query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)

            if searchedOnce {
                Button {
                    Task { await runSearch(forceRefresh: true) }
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.subheadline.weight(.semibold))
                }
                .buttonStyle(.bordered)
                .disabled(loading)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private var historyStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                ForEach(history, id: \.self) { term in
                    Button {
                        query = term
                        Task { await runSearch(forceRefresh: false) }
                    } label: {
                        Text(term)
                            .font(.caption)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(.quaternary.opacity(0.5), in: Capsule())
                            .foregroundStyle(.primary)
                    }
                    .buttonStyle(.plain)
                }
                Button("清空") {
                    history = []
                    UserDefaults.standard.removeObject(forKey: historyKey)
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 14)
        }
        .padding(.bottom, 6)
    }

    private func resultCard(_ item: PerformerRecentRelease) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                Color.black.opacity(0.88)
                if let raw = item.coverUrl, !raw.isEmpty,
                   let url = model.proxyScraperImageURL(rawUrl: raw) {
                    CachedImageView(url: url, contentMode: .fit, minHeight: 104, maxPixel: 640)
                } else {
                    Image(systemName: "film")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                }
            }
            .overlay(alignment: .topLeading) {
                HStack {
                    Text(item.number)
                        .font(.system(size: 9, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.black.opacity(0.75), in: RoundedRectangle(cornerRadius: 4))
                        .foregroundStyle(.white)
                    Spacer()
                    Text(item.inLibrary ? "已入库" : "看详情")
                        .font(.system(size: 9, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background((item.inLibrary ? Color.green : Color.purple).opacity(0.88), in: RoundedRectangle(cornerRadius: 4))
                        .foregroundStyle(.white)
                }
                .padding(6)
            }
            .aspectRatio(3 / 2, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

            Text(item.title.isEmpty ? item.number : item.title)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.primary)
                .lineLimit(2)
                .multilineTextAlignment(.leading)

            HStack {
                Text(item.releaseDate ?? "未知日期")
                Spacer()
                Text(item.source)
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
        }
    }

    private func consumePresetIfNeeded() {
        let trimmed = presetQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        presetQuery = ""
        query = trimmed
        Task { await runSearch(forceRefresh: false) }
    }

    private func loadHistory() {
        history = UserDefaults.standard.stringArray(forKey: historyKey) ?? []
    }

    private func saveHistory(_ term: String) {
        let clean = term.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        var next = history.filter { $0.caseInsensitiveCompare(clean) != .orderedSame }
        next.insert(clean, at: 0)
        history = Array(next.prefix(10))
        UserDefaults.standard.set(history, forKey: historyKey)
    }

    private func runSearch(forceRefresh: Bool) async {
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let client = model.apiClient else { return }
        loading = true
        searchedOnce = true
        errorMessage = nil
        saveHistory(trimmed)
        do {
            results = try await client.searchOnlineMovies(query: trimmed, forceRefresh: forceRefresh)
        } catch {
            errorMessage = error.localizedDescription
            results = []
        }
        loading = false
    }
}

/// 线上作品完整详情页：高清封面 + 演员/片厂/标签 + 剧照画廊（可大图滑动）+ 内嵌磁力列表
struct OnlineMovieDetailSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let release: PerformerRecentRelease
    var onPlayAsset: ((String, String) -> Void)? = nil
    var onFilterActor: ((String) -> Void)? = nil

    @State private var detail: MovieMetadata?
    @State private var detailLoading = true
    @State private var detailError: String?

    @State private var magnets: [MovieMagnetItem] = []
    @State private var magnetsLoading = true
    @State private var magnetsError: String?

    @State private var copiedMagnet: String?
    @State private var copyToast: String?
    @State private var activeSampleIndex: Int?

    private var displayNumber: String { detail?.number ?? release.number }
    private var displayTitle: String {
        let t = detail?.title ?? release.title
        return t.isEmpty ? release.number : t
    }
    private var displayCover: String? { detail?.coverUrl ?? release.coverUrl }
    private var displayDate: String? { detail?.releaseDate ?? release.releaseDate }
    private var isInLibrary: Bool { release.inLibrary || (detail?.assetId != nil) }
    private var samples: [String] { detail?.sampleImages ?? [] }

    var body: some View {
        NavigationStack {
            ZStack(alignment: .top) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        coverSection
                        headerInfoSection
                        actionButtonsSection
                        metadataFactsSection
                        actorsAndGenresSection
                        samplesSection
                        magnetsSection
                    }
                    .padding(16)
                }

                if let copyToast {
                    Text(copyToast)
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .background(.ultraThinMaterial, in: Capsule())
                        .padding(.top, 10)
                        .transition(.opacity)
                }
            }
            .navigationTitle(displayNumber)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
            .task {
                async let metaTask: Void = loadDetail()
                async let magnetTask: Void = loadMagnets(forceRefresh: false)
                _ = await (metaTask, magnetTask)
            }
            .fullScreenCover(isPresented: Binding(
                get: { activeSampleIndex != nil },
                set: { if !$0 { activeSampleIndex = nil } }
            )) {
                OnlineSampleGalleryViewer(
                    urls: samples.compactMap { model.proxyScraperImageURL(rawUrl: $0) },
                    initialIndex: activeSampleIndex ?? 0
                )
            }
        }
    }

    private var coverSection: some View {
        ZStack {
            Color.black
            if let raw = displayCover, !raw.isEmpty,
               let url = model.proxyScraperImageURL(rawUrl: raw) {
                CachedImageView(url: url, contentMode: .fit, minHeight: 200, maxPixel: 1200)
            } else {
                Image(systemName: "film")
                    .font(.largeTitle)
                    .foregroundStyle(.secondary)
            }
        }
        .aspectRatio(3 / 2, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .shadow(color: .black.opacity(0.28), radius: 8, x: 0, y: 4)
    }

    private var headerInfoSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Button {
                    UIPasteboard.general.string = displayNumber
                    showToast("已复制番号 \(displayNumber)")
                } label: {
                    HStack(spacing: 4) {
                        Text(displayNumber).font(.caption.bold())
                        Image(systemName: "doc.on.doc").font(.system(size: 9))
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(LinearGradient(colors: [.purple, .pink], startPoint: .leading, endPoint: .trailing), in: RoundedRectangle(cornerRadius: 6))
                    .foregroundStyle(.white)
                }
                .buttonStyle(.plain)

                Text(isInLibrary ? "已在本地库" : "线上资源")
                    .font(.caption2.bold())
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background((isInLibrary ? Color.green : Color.purple).opacity(0.18), in: Capsule())
                    .foregroundStyle(isInLibrary ? .green : .purple)

                if let date = displayDate, !date.isEmpty {
                    Text(date)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let dur = detail?.durationMinutes, dur > 0 {
                    Text("\(dur) 分钟")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Text(displayTitle)
                .font(.headline)
                .fixedSize(horizontal: false, vertical: true)

            if detailLoading {
                HStack(spacing: 6) {
                    ProgressView().scaleEffect(0.75)
                    Text("正在补全演员、片厂与剧照…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else if let detailError {
                Text("详细元数据未完全拉取：\(detailError)")
                    .font(.caption2)
                    .foregroundStyle(.orange)
            }
        }
    }

    @ViewBuilder
    private var actionButtonsSection: some View {
        if let assetId = detail?.assetId, !assetId.isEmpty {
            Button {
                onPlayAsset?(assetId, displayTitle)
            } label: {
                Label("立即播放本地视频", systemImage: "play.fill")
                    .font(.subheadline.bold())
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
            }
            .buttonStyle(.borderedProminent)
            .tint(.purple)
        }
    }

    @ViewBuilder
    private var metadataFactsSection: some View {
        if let d = detail {
            let pairs: [(String, String)] = [
                ("片厂", d.maker ?? ""),
                ("发行商", d.label ?? ""),
                ("导演", d.director ?? ""),
                ("系列", d.series ?? ""),
            ].filter { !$0.1.isEmpty }
            if !pairs.isEmpty {
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], alignment: .leading, spacing: 8) {
                    ForEach(pairs, id: \.0) { label, value in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(label).font(.caption2).foregroundStyle(.secondary)
                            Text(value).font(.caption.weight(.semibold)).lineLimit(2)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var actorsAndGenresSection: some View {
        if let actors = detail?.actors, !actors.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("参演演员")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(actors, id: \.self) { actor in
                            Button {
                                onFilterActor?(actor)
                            } label: {
                                Label(actor, systemImage: "person.fill")
                                    .font(.caption.weight(.semibold))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(Color.purple.opacity(0.16), in: Capsule())
                                    .foregroundStyle(.purple)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }

        if let tags = detail?.tags, !tags.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("题材标签")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(tags, id: \.self) { tag in
                            Text(tag)
                                .font(.caption2)
                                .padding(.horizontal, 9)
                                .padding(.vertical, 4)
                                .background(.quaternary.opacity(0.4), in: Capsule())
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var samplesSection: some View {
        if !samples.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Text("剧照预览（\(samples.count) 张 · 点击放大）")
                    .font(.caption.bold())
                    .foregroundStyle(.secondary)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(Array(samples.enumerated()), id: \.offset) { idx, raw in
                            Button {
                                activeSampleIndex = idx
                            } label: {
                                ZStack {
                                    Color.black.opacity(0.85)
                                    if let url = model.proxyScraperImageURL(rawUrl: raw) {
                                        CachedImageView(url: url, contentMode: .fill, minHeight: 96, maxPixel: 540)
                                    }
                                }
                                .frame(width: 144, height: 96)
                                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
    }

    private var magnetsSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label("磁力资源\(magnetsLoading ? "" : " (\(magnets.count))")", systemImage: "link")
                    .font(.subheadline.bold())
                Spacer()
                Button {
                    Task { await loadMagnets(forceRefresh: true) }
                } label: {
                    Label("刷新", systemImage: "arrow.clockwise")
                        .font(.caption.weight(.semibold))
                }
                .disabled(magnetsLoading)
            }

            if let magnetsError {
                Label(magnetsError, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            if magnetsLoading {
                HStack(spacing: 8) {
                    ProgressView()
                    Text("正在检索可用磁力链接…")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)
            } else if magnets.isEmpty {
                Text("暂未找到该番号的公开磁力链接")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 16)
                    .background(.quaternary.opacity(0.2), in: RoundedRectangle(cornerRadius: 10))
            } else {
                ForEach(magnets) { item in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(item.title)
                            .font(.caption.weight(.semibold))
                            .lineLimit(3)
                        HStack(spacing: 6) {
                            ForEach(item.tags, id: \.self) { tag in
                                Text(tag)
                                    .font(.system(size: 10, weight: .bold))
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(
                                        (tag.contains("字幕") ? Color.pink : (tag.contains("高清") ? Color.blue : Color.secondary)).opacity(0.18),
                                        in: Capsule()
                                    )
                                    .foregroundStyle(tag.contains("字幕") ? .pink : (tag.contains("高清") ? .blue : .secondary))
                            }
                            if let size = item.size, !size.isEmpty {
                                Text(size).font(.caption2).foregroundStyle(.secondary)
                            }
                            if let seeders = item.seeders {
                                Text("做种 \(seeders)").font(.caption2).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Text(item.source).font(.caption2).foregroundStyle(.tertiary)
                        }
                        Button {
                            UIPasteboard.general.string = item.magnet
                            copiedMagnet = item.magnet
                            showToast("已复制磁力链接")
                        } label: {
                            Label(copiedMagnet == item.magnet ? "已复制磁力" : "复制磁力链接", systemImage: copiedMagnet == item.magnet ? "checkmark" : "doc.on.doc")
                                .font(.caption.bold())
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(copiedMagnet == item.magnet ? .green : .purple)
                        .controlSize(.small)
                    }
                    .padding(12)
                    .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
                }
            }
        }
    }

    private func showToast(_ text: String) {
        copyToast = text
        Task {
            try? await Task.sleep(nanoseconds: 1_800_000_000)
            if copyToast == text { copyToast = nil }
        }
    }

    private func loadDetail() async {
        guard let client = model.apiClient else { detailLoading = false; return }
        detailLoading = true
        detailError = nil
        do {
            detail = try await client.getOnlineMovieDetail(number: release.number)
        } catch {
            detailError = error.localizedDescription
        }
        detailLoading = false
    }

    private func loadMagnets(forceRefresh: Bool) async {
        guard let client = model.apiClient else { magnetsLoading = false; return }
        magnetsLoading = true
        magnetsError = nil
        do {
            magnets = try await client.getMovieMagnets(number: release.number, detailUrl: release.detailUrl, forceRefresh: forceRefresh)
        } catch {
            magnetsError = error.localizedDescription
        }
        magnetsLoading = false
    }
}

private struct OnlineSampleGalleryViewer: View {
    let urls: [URL]
    @State private var index: Int
    @Environment(\.dismiss) private var dismiss

    init(urls: [URL], initialIndex: Int) {
        self.urls = urls
        _index = State(initialValue: min(max(0, initialIndex), max(0, urls.count - 1)))
    }

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.ignoresSafeArea()
            if !urls.isEmpty {
                TabView(selection: $index) {
                    ForEach(Array(urls.enumerated()), id: \.offset) { idx, url in
                        CachedImageView(url: url, contentMode: .fit, minHeight: 240, maxPixel: 2048)
                            .tag(idx)
                    }
                }
                .tabViewStyle(.page(indexDisplayMode: .automatic))
            }

            HStack {
                Text("\(index + 1) / \(urls.count)")
                    .font(.caption.bold())
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(.black.opacity(0.6), in: Capsule())
                    .foregroundStyle(.white)
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 28))
                        .foregroundStyle(.white.opacity(0.88))
                }
            }
            .padding(20)
        }
    }
}
