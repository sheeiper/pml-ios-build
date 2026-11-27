import SwiftUI

public struct MovieDetailSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var appModel: AppModel
    @ObservedObject private var favs = EntityFavoritesStore.shared

    public let number: String?
    public let assetId: String?
    public let fileName: String?
    public let remotePath: String?
    public let onPlay: ((String) -> Void)?
    public let onFilterActor: ((String) -> Void)?
    public let onFilterMaker: ((String) -> Void)?

    @State private var loading = false
    @State private var errorMessage: String?
    @State private var metadata: MovieMetadata?
    @State private var parts: [MovieAssetPart] = []
    @State private var relatedGrouped: RelatedMoviesGrouped?
    @State private var activeFullScreen: DetailFullScreen?
    @State private var resolvingPlayback = false
    @State private var playbackErrorText: String?
    @State private var selectedPerformer: MoviePerformer?

    private enum DetailFullScreen: Identifiable {
        case sample(Int)
        case player(PreviewItem)

        var id: String {
            switch self {
            case .sample(let idx): return "sample-\(idx)"
            case .player(let item): return "player-\(item.id)"
            }
        }
    }
    @State private var currentNumber: String?
    @State private var savedSubtitles: [SavedMovieSubtitle] = []
    @State private var subtitleCandidates: [SubtitleSearchCandidate] = []
    @State private var subtitleSearching = false
    @State private var subtitleSearchDone = false
    @State private var subtitleError: String?
    @State private var subtitleDownloadingId: String?
    @State private var subtitleDeletingId: String?
    @State private var copyToast: String?
    @State private var showNoAssetAlert = false
    @State private var showMultiSourceSheet = false
    @State private var showMoreVersionsSheet = false
    @State private var activeSeriesSheet: String?

    public init(
        number: String? = nil,
        assetId: String? = nil,
        fileName: String? = nil,
        remotePath: String? = nil,
        onPlay: ((String) -> Void)? = nil,
        onFilterActor: ((String) -> Void)? = nil,
        onFilterMaker: ((String) -> Void)? = nil
    ) {
        self.number = number
        self.assetId = assetId
        self.fileName = fileName
        self.remotePath = remotePath
        self.onPlay = onPlay
        self.onFilterActor = onFilterActor
        self.onFilterMaker = onFilterMaker
    }

    public var body: some View {
        NavigationStack {
            contentStack
                .sheet(item: $selectedPerformer) { person in
                    performerSheet(person)
                }
                .sheet(isPresented: $showMoreVersionsSheet) {
                    moreVersionsSheet
                }
                .sheet(isPresented: $showMultiSourceSheet) {
                    multiSourceSheet
                }
                .alert("未找到本地视频文件", isPresented: $showNoAssetAlert) {
                    Button("知道了", role: .cancel) {}
                } message: {
                    Text("该作品当前仅有影视百科资料，网盘或本地尚未匹配到对应的视频文件。可在影视库【数据】中执行增量同步。")
                }
                .alert("无法播放视频", isPresented: Binding(
                    get: { playbackErrorText != nil },
                    set: { if !$0 { playbackErrorText = nil } }
                )) {
                    Button("知道了", role: .cancel) { playbackErrorText = nil }
                } message: {
                    Text(playbackErrorText ?? "")
                }
                .sheet(isPresented: Binding(
                    get: { activeSeriesSheet != nil },
                    set: { if !$0 { activeSeriesSheet = nil } }
                )) {
                    seriesSheet
                }
                .fullScreenCover(item: $activeFullScreen) { modal in
                    fullScreenModal(modal)
                }
                .task {
                    currentNumber = number
                    await loadData()
                }
        }
    }

    private var contentStack: some View {
        ZStack(alignment: .top) {
            mainScrollView

            if let toast = copyToast {
                Text(toast)
                    .font(.caption.bold())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(.black.opacity(0.85), in: Capsule())
                    .padding(.top, 10)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .zIndex(100)
            }
        }
        .animation(.easeInOut(duration: 0.2), value: copyToast)
        .navigationTitle(metadata?.number ?? "影视百科详情")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                if let favNumber = metadata?.number ?? currentNumber ?? number, !favNumber.isEmpty {
                    let isFav = favs.isFavorited(.movie, id: favNumber)
                    Button {
                        Task { await favs.toggle(.movie, id: favNumber, client: appModel.apiClient) }
                    } label: {
                        Image(systemName: isFav ? "heart.fill" : "heart")
                            .foregroundStyle(isFav ? .red : .primary)
                    }
                    .accessibilityLabel(isFav ? "取消收藏影片" : "收藏影片")
                }
                Button("完成") { dismiss() }
            }
        }
        .task {
            await favs.ensureLoaded(client: appModel.apiClient)
        }
    }

    @ViewBuilder
    private func performerSheet(_ person: MoviePerformer) -> some View {
        MoviePerformerDetailSheet(
            performer: person,
            onFilterMovies: onFilterActor.map { handler in { actorName in selectedPerformer = nil; dismiss(); handler(actorName) } },
            onPlay: onPlay.map { handler in
                { targetId, _ in
                    selectedPerformer = nil
                    dismiss()
                    handler(targetId)
                }
            }
        )
        .environmentObject(appModel)
    }

    @ViewBuilder
    private var moreVersionsSheet: some View {
        if let targetNum = metadata?.number ?? currentNumber ?? number {
            OnlineMovieDetailSheet(
                release: PerformerRecentRelease(
                    number: targetNum,
                    title: metadata?.title ?? targetNum,
                    releaseDate: metadata?.releaseDate,
                    coverUrl: metadata?.coverUrl,
                    detailUrl: "",
                    inLibrary: true,
                    source: metadata?.source ?? "local"
                ),
                onPlayAsset: { _, _ in showMoreVersionsSheet = false },
                onFilterActor: { _ in showMoreVersionsSheet = false }
            )
            .environmentObject(appModel)
        }
    }

    @ViewBuilder
    private var multiSourceSheet: some View {
        if let targetNum = metadata?.number ?? currentNumber ?? number {
            MovieMultiSourceSheet(number: targetNum, currentSource: metadata?.source) { saved in
                metadata = saved
                showMultiSourceSheet = false
            }
            .environmentObject(appModel)
        }
    }

    @ViewBuilder
    private var seriesSheet: some View {
        if let series = activeSeriesSheet {
            MovieSeriesSheet(
                series: series,
                onOpenLocalMovie: { movie in
                    activeSeriesSheet = nil
                    metadata = movie
                    currentNumber = movie.number
                    Task { await loadAux(for: movie.number) }
                },
                onPlayAsset: { targetId, title in
                    activeSeriesSheet = nil
                    Task { await playLocally(assetId: targetId, title: title) }
                }
            )
            .environmentObject(appModel)
        }
    }

    @ViewBuilder
    private func fullScreenModal(_ modal: DetailFullScreen) -> some View {
        switch modal {
        case .sample(let idx):
            SampleGalleryPagerView(
                images: metadata?.sampleImages ?? [],
                initialIndex: idx,
                onClose: { activeFullScreen = nil }
            )
            .environmentObject(appModel)
        case .player(let item):
            GestureVideoPlayerSheet(
                url: item.url,
                title: item.title,
                assetId: item.assetId,
                startAt: item.startAt,
                fallbackURL: item.fallbackURL,
                sourceLabel: item.sourceLabel,
                remotePath: item.remotePath
            )
            .id(item.id)
            .environmentObject(appModel)
        }
    }

    @ViewBuilder
    private var mainScrollView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                    if loading {
                        HStack {
                            Spacer()
                            VStack(spacing: 12) {
                                ProgressView().scaleEffect(1.2)
                                Text("正在联网刮削影视百科与剧照...")
                                    .font(.footnote)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                        }
                        .padding(.vertical, 60)
                    } else if let error = errorMessage {
                        VStack(spacing: 12) {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .font(.system(size: 36))
                                .foregroundStyle(.orange)
                            Text(error)
                                .font(.subheadline)
                                .multilineTextAlignment(.center)
                                .foregroundStyle(.secondary)
                            Button {
                                showMultiSourceSheet = true
                            } label: {
                                Image(systemName: "square.stack.3d.up")
                                    .font(.headline)
                                    .padding(.vertical, 8)
                                    .padding(.horizontal, 12)
                            }
                            .buttonStyle(.bordered)

                            Button {
                                Task { await loadData(force: true) }
                            } label: {
                                Label("重试刮削", systemImage: "arrow.clockwise")
                                    .font(.subheadline.bold())
                            }
                            .buttonStyle(.bordered)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 40)
                    } else if let meta = metadata {
                        // Full-width 3:2 cover (contain so FC2 irregular covers never crop)
                        ZStack {
                            Color.black
                            CachedImageView(
                                url: meta.coverUrl.flatMap { appModel.proxyScraperImageURL(rawUrl: $0) },
                                contentMode: .fit,
                                minHeight: 200,
                                maxPixel: 1200
                            )
                        }
                        .aspectRatio(3 / 2, contentMode: .fit)
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                        .shadow(color: .black.opacity(0.32), radius: 10, x: 0, y: 4)

                        // Title & Badges
                        VStack(alignment: .leading, spacing: 8) {
                            HStack(spacing: 6) {
                                Button {
                                    UIPasteboard.general.string = meta.number
                                    copyToast = "已复制番号 \(meta.number)"
                                    Task {
                                        try? await Task.sleep(nanoseconds: 1_800_000_000)
                                        copyToast = nil
                                    }
                                } label: {
                                    HStack(spacing: 4) {
                                        Text(meta.number)
                                            .font(.caption.bold())
                                        Image(systemName: "doc.on.doc")
                                            .font(.system(size: 9))
                                    }
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(LinearGradient(colors: [.purple, .pink], startPoint: .leading, endPoint: .trailing))
                                    .foregroundStyle(.white)
                                    .clipShape(RoundedRectangle(cornerRadius: 5))
                                }
                                .buttonStyle(.plain)

                                if let date = meta.releaseDate, !date.isEmpty {
                                    HStack(spacing: 3) {
                                        Image(systemName: "calendar")
                                            .font(.system(size: 9))
                                        Text("发布：\(date)")
                                    }
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                }

                                if meta.updatedAt > 0 {
                                    HStack(spacing: 3) {
                                        Image(systemName: "arrow.down.circle")
                                            .font(.system(size: 9))
                                        Text("入库：\(formatTimestamp(meta.updatedAt))")
                                    }
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                }

                                if let duration = meta.durationMinutes, duration > 0 {
                                    Text("\(duration) 分钟")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }

                                if parts.count > 1 {
                                    Text("\(parts.count) 分片")
                                        .font(.caption2.bold())
                                        .padding(.horizontal, 6)
                                        .padding(.vertical, 2)
                                        .background(Color.blue.opacity(0.16), in: Capsule())
                                        .foregroundStyle(.blue)
                                }
                            }

                            Text(meta.title)
                                .font(.headline)
                                .foregroundStyle(.primary)

                            if let orig = meta.originalTitle, !orig.isEmpty, orig != meta.title {
                                Text(orig)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                        }

                        // Play & Actions (2 clean rows so Play & Subtitle are never truncated)
                        VStack(spacing: 8) {
                            HStack(spacing: 10) {
                                Button {
                                    if let targetId = parts.first?.assetId ?? meta.assetId ?? assetId {
                                        Task { await playLocally(assetId: targetId, title: meta.title) }
                                    } else {
                                        showNoAssetAlert = true
                                    }
                                } label: {
                                    HStack(spacing: 6) {
                                        if resolvingPlayback {
                                            ProgressView().tint(.white)
                                        } else {
                                            Image(systemName: "play.fill")
                                        }
                                        Text(resolvingPlayback ? "解析中…" : "立即播放")
                                            .lineLimit(1)
                                    }
                                    .font(.headline)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 8)
                                }
                                .buttonStyle(.borderedProminent)
                                .tint(.purple)
                                .disabled(resolvingPlayback)

                                Button {
                                    Task { await searchSubtitles(for: meta.number) }
                                } label: {
                                    HStack(spacing: 5) {
                                        if subtitleSearching {
                                            ProgressView()
                                        } else {
                                            Image(systemName: "captions.bubble")
                                        }
                                        Text("字幕")
                                            .lineLimit(1)
                                    }
                                    .font(.subheadline.bold())
                                    .padding(.vertical, 8)
                                    .padding(.horizontal, 12)
                                }
                                .buttonStyle(.bordered)
                                .disabled(subtitleSearching)
                            }

                            HStack(spacing: 8) {
                                let isMovieFav = favs.isMovieFav(meta.number)
                                Button {
                                    Task { await favs.toggle(client: appModel.apiClient, type: .movie, id: meta.number) }
                                } label: {
                                    Label(isMovieFav ? "已收藏" : "收藏", systemImage: isMovieFav ? "heart.fill" : "heart")
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(isMovieFav ? .pink : .primary)
                                        .frame(maxWidth: .infinity)
                                        .padding(.vertical, 6)
                                }
                                .buttonStyle(.bordered)

                                Button {
                                    showMoreVersionsSheet = true
                                } label: {
                                    Label("更多版本", systemImage: "link")
                                        .font(.caption.weight(.semibold))
                                        .frame(maxWidth: .infinity)
                                        .padding(.vertical, 6)
                                }
                                .buttonStyle(.bordered)
                                .tint(.purple)

                                Button {
                                    showMultiSourceSheet = true
                                } label: {
                                    Label("多源资料", systemImage: "square.stack.3d.up")
                                        .font(.caption.weight(.semibold))
                                        .frame(maxWidth: .infinity)
                                        .padding(.vertical, 6)
                                }
                                .buttonStyle(.bordered)

                                Button {
                                    Task { await loadData(force: true) }
                                } label: {
                                    Label("刷新", systemImage: "arrow.clockwise")
                                        .font(.caption.weight(.semibold))
                                        .frame(maxWidth: .infinity)
                                        .padding(.vertical, 6)
                                }
                                .buttonStyle(.bordered)
                            }
                        }

                        // Chinese Subtitle Management (zh-CN / zh-TW only)
                        if !savedSubtitles.isEmpty || subtitleSearching || subtitleSearchDone || subtitleError != nil {
                            subtitleSection(number: meta.number)
                        }

                        // Multi-part / local file switcher
                        if !parts.isEmpty {
                            VStack(alignment: .leading, spacing: 6) {
                                Text(parts.count > 1 ? "多文件版本 / 分段（共 \(parts.count) 个文件）" : "关联本地视频")
                                    .font(.footnote.bold())
                                    .foregroundStyle(.secondary)
                                ForEach(Array(parts.enumerated()), id: \.element.id) { idx, part in
                                    Button {
                                        Task { await playLocally(assetId: part.assetId, title: part.fileName) }
                                    } label: {
                                        HStack(spacing: 8) {
                                            Image(systemName: "play.circle.fill")
                                                .foregroundStyle(.purple)
                                            if parts.count > 1 {
                                                Text("P\(idx + 1)")
                                                    .font(.caption2.bold())
                                                    .padding(.horizontal, 6)
                                                    .padding(.vertical, 2)
                                                    .background(Color.purple.opacity(0.2), in: RoundedRectangle(cornerRadius: 4))
                                                    .foregroundStyle(.purple)
                                            }
                                            VStack(alignment: .leading, spacing: 3) {
                                                Text(part.fileName)
                                                    .font(.caption.weight(.semibold))
                                                    .lineLimit(1)
                                                    .foregroundStyle(.primary)
                                                if let badges = part.badges, !badges.isEmpty {
                                                    HStack(spacing: 4) {
                                                        ForEach(badges, id: \.self) { b in
                                                            Text(b)
                                                                .font(.system(size: 10, weight: .bold))
                                                                .padding(.horizontal, 5)
                                                                .padding(.vertical, 1.5)
                                                                .background(partBadgeColor(b).opacity(0.2), in: Capsule())
                                                                .foregroundStyle(partBadgeColor(b))
                                                        }
                                                    }
                                                } else {
                                                    Text(part.remotePath)
                                                        .font(.system(size: 10))
                                                        .lineLimit(1)
                                                        .foregroundStyle(.secondary)
                                                }
                                            }
                                            Spacer()
                                            if part.sizeBytes > 0 {
                                                Text(formatBytes(part.sizeBytes))
                                                    .font(.caption2)
                                                    .foregroundStyle(.secondary)
                                            }
                                        }
                                        .padding(10)
                                        .background(.quaternary.opacity(0.22), in: RoundedRectangle(cornerRadius: 10))
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }

                        // Structured Metadata Specs
                        let specs = buildSpecs(meta)
                        if !specs.isEmpty {
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                                ForEach(specs, id: \.label) { spec in
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(spec.label).font(.caption2).foregroundStyle(.secondary)
                                        if spec.isMaker, let maker = meta.maker, let onFilterMaker {
                                            Button(maker) {
                                                dismiss()
                                                onFilterMaker(maker)
                                            }
                                            .font(.caption.weight(.semibold))
                                            .foregroundStyle(.purple)
                                        } else if spec.isSeries, let series = meta.series, !series.isEmpty {
                                            Button {
                                                activeSeriesSheet = series
                                            } label: {
                                                HStack(spacing: 4) {
                                                    Text(series).lineLimit(2)
                                                    Image(systemName: "chevron.right").font(.caption2)
                                                }
                                                .font(.caption.weight(.semibold))
                                                .foregroundStyle(.purple)
                                            }
                                        } else {
                                            Text(spec.value)
                                                .font(.caption.weight(.semibold))
                                                .lineLimit(2)
                                        }
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(10)
                                    .background(.quaternary.opacity(0.2), in: RoundedRectangle(cornerRadius: 9))
                                }
                            }
                        }

                        // Clickable Actresses
                        if !meta.actors.isEmpty {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(spacing: 4) {
                                    Image(systemName: "person.2.fill")
                                        .font(.caption2)
                                        .foregroundStyle(.pink)
                                    Text("主演阵容（点击查看档案）")
                                        .font(.footnote.bold())
                                        .foregroundStyle(.secondary)
                                }

                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 8) {
                                        ForEach(meta.actors, id: \.self) { actress in
                                            let age = ageAtRelease(for: actress, releaseDate: meta.releaseDate)
                                            Button {
                                                Task { await openPerformer(named: actress) }
                                            } label: {
                                                Label(age.map { "\(actress) · \($0)岁" } ?? actress, systemImage: "person.circle.fill")
                                                    .font(.caption.weight(.semibold))
                                                    .padding(.horizontal, 10)
                                                    .padding(.vertical, 6)
                                                    .background(Color.pink.opacity(0.14))
                                                    .foregroundStyle(Color.pink)
                                                    .clipShape(Capsule())
                                            }
                                            .buttonStyle(.plain)
                                        }
                                    }
                                }
                            }
                        }

                        // Tags
                        if !meta.tags.isEmpty {
                            VStack(alignment: .leading, spacing: 6) {
                                Text("题材标签")
                                    .font(.footnote.bold())
                                    .foregroundStyle(.secondary)

                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 6) {
                                        ForEach(meta.tags, id: \.self) { tag in
                                            Text("# \(tag)")
                                                .font(.caption2)
                                                .padding(.horizontal, 8)
                                                .padding(.vertical, 4)
                                                .background(Color.secondary.opacity(0.12))
                                                .foregroundStyle(.secondary)
                                                .clipShape(Capsule())
                                        }
                                    }
                                }
                            }
                        }

                        // Sample Images
                        if !meta.sampleImages.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("剧照画廊 (\(meta.sampleImages.count))")
                                    .font(.footnote.bold())
                                    .foregroundStyle(.secondary)

                                ScrollView(.horizontal, showsIndicators: false) {
                                    LazyHStack(spacing: 8) {
                                        ForEach(Array(meta.sampleImages.enumerated()), id: \.offset) { idx, sample in
                                            if let url = appModel.proxyScraperImageURL(rawUrl: sample) {
                                                Button {
                                                    activeFullScreen = .sample(idx)
                                                } label: {
                                                    CachedImageView(url: url, contentMode: .fill, minHeight: 100, maxPixel: 540)
                                                        .frame(width: 150, height: 100)
                                                        .clipShape(RoundedRectangle(cornerRadius: 8))
                                                }
                                                .buttonStyle(.plain)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Partitioned Related Movies: Same Actors vs Same Maker
                        if let grouped = relatedGrouped {
                            if !grouped.byActors.isEmpty {
                                relatedSection(
                                    title: "同演员更多作品",
                                    subtitle: grouped.actorNames.prefix(2).joined(separator: " · "),
                                    items: grouped.byActors,
                                    filterAction: grouped.actorNames.first.flatMap { name in
                                        onFilterActor.map { handler in { dismiss(); handler(name) } }
                                    }
                                )
                            }
                            if !grouped.byMaker.isEmpty {
                                relatedSection(
                                    title: "同厂商推荐",
                                    subtitle: grouped.makerName ?? "",
                                    items: grouped.byMaker,
                                    filterAction: grouped.makerName.flatMap { maker in
                                        onFilterMaker.map { handler in { dismiss(); handler(maker) } }
                                    }
                                )
                            }
                        }
                    }
                }
                .padding()
            }
    }

    private func relatedSection(title: String, subtitle: String, items: [MovieMetadata], filterAction: (() -> Void)?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(title).font(.footnote.bold()).foregroundStyle(.secondary)
                if !subtitle.isEmpty {
                    Text(subtitle).font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
                }
                Spacer()
                if let filterAction {
                    Button("查看全部", action: filterAction)
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.purple)
                }
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(items) { rel in
                        Button {
                            metadata = rel
                            currentNumber = rel.number
                            Task { await loadAux(for: rel.number) }
                        } label: {
                            VStack(alignment: .leading, spacing: 4) {
                                ZStack {
                                    Color.black.opacity(0.85)
                                    if let cover = rel.coverUrl, let url = appModel.proxyScraperImageURL(rawUrl: cover) {
                                        CachedImageView(url: url, contentMode: .fit, minHeight: 84, maxPixel: 400)
                                    }
                                }
                                .frame(width: 126, height: 84)
                                .clipShape(RoundedRectangle(cornerRadius: 8))

                                Text(rel.number)
                                    .font(.caption2.bold())
                                    .foregroundStyle(.primary)
                                Text(rel.title)
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .frame(width: 126, alignment: .leading)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private struct SpecItem { let label: String; let value: String; var isMaker: Bool = false; var isSeries: Bool = false }

    private func buildSpecs(_ meta: MovieMetadata) -> [SpecItem] {
        var list: [SpecItem] = []
        if let maker = meta.maker, !maker.isEmpty { list.append(SpecItem(label: "片商", value: maker, isMaker: true)) }
        if let label = meta.label, !label.isEmpty { list.append(SpecItem(label: "发行厂牌", value: label)) }
        if let series = meta.series, !series.isEmpty { list.append(SpecItem(label: "所属系列", value: series, isSeries: true)) }
        if let director = meta.director, !director.isEmpty { list.append(SpecItem(label: "导演", value: director)) }
        list.append(SpecItem(label: "数据来源", value: meta.source.isEmpty ? "未知" : meta.source))
        return list
    }

    private func partBadgeColor(_ badge: String) -> Color {
        if badge.contains("LTX") { return .orange }
        if badge.contains("特典") || badge.contains("花絮") { return .yellow }
        if badge.contains("重复") { return .red }
        if badge.contains("中字") || badge.contains("破解") { return .green }
        if badge.contains("4K") || badge.contains("1080P") || badge.contains("AI修复") { return .blue }
        return .purple
    }

    private func formatBytes(_ bytes: Int64) -> String {
        let gb = Double(bytes) / 1_073_741_824.0
        if gb >= 1.0 { return String(format: "%.2f GB", gb) }
        return "\(bytes / 1_048_576) MB"
    }

    private func formatTimestamp(_ ts: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(ts))
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm"
        return f.string(from: date)
    }

    private func openPerformer(named name: String) async {
        guard let client = appModel.apiClient else { return }
        if let list = try? await client.listMoviePerformers(query: name, limit: 10, offset: 0) {
            let exact = list.items.first(where: { $0.name == name || $0.aliases.contains(name) })
            selectedPerformer = exact ?? list.items.first
        }
    }

    private func playLocally(assetId: String, title: String) async {
        guard !resolvingPlayback else { return }
        resolvingPlayback = true
        defer { resolvingPlayback = false }
        guard let source = await appModel.playbackSource(assetId: assetId, remotePath: remotePath) else {
            playbackErrorText = appModel.playbackError ?? "无法解析视频直链，请检查服务器或 AList 配置"
            return
        }
        activeFullScreen = .player(PreviewItem(
            url: source.url,
            isVideo: true,
            assetId: assetId,
            title: title,
            fallbackURL: source.fallbackURL,
            sourceLabel: source.label,
            remotePath: remotePath
        ))
    }

    private func ageAtRelease(for actress: String, releaseDate: String?) -> Int? {
        guard let birthRaw = relatedGrouped?.actorBirthDates?[actress],
              let relRaw = releaseDate else { return nil }
        let bParts = birthRaw.split(whereSeparator: { $0 == "-" || $0 == "/" }).compactMap { Int($0) }
        let rParts = relRaw.split(whereSeparator: { $0 == "-" || $0 == "/" }).compactMap { Int($0) }
        guard bParts.count >= 3, rParts.count >= 3 else { return nil }
        var age = rParts[0] - bParts[0]
        if rParts[1] < bParts[1] || (rParts[1] == bParts[1] && rParts[2] < bParts[2]) {
            age -= 1
        }
        return (14...85).contains(age) ? age : nil
    }

    private func loadAux(for num: String) async {
        guard let client = appModel.apiClient else { return }
        subtitleCandidates = []
        subtitleSearchDone = false
        subtitleError = nil
        async let fetchedParts = try? client.listMovieAssets(number: num)
        async let fetchedRelated = try? client.getRelatedMoviesGrouped(number: num, limitPerGroup: 8)
        async let fetchedSubs = try? client.listMovieSubtitles(number: num)
        parts = (await fetchedParts) ?? []
        relatedGrouped = await fetchedRelated
        savedSubtitles = (await fetchedSubs) ?? []
    }

    private func subtitleLangLabel(_ lang: String) -> String {
        switch lang {
        case "zh-CN": return "简体中文"
        case "zh-TW": return "繁体中文"
        default: return lang.isEmpty ? "中文" : lang
        }
    }

    private func searchSubtitles(for num: String) async {
        guard let client = appModel.apiClient, !subtitleSearching else { return }
        subtitleSearching = true
        subtitleSearchDone = false
        subtitleError = nil
        subtitleCandidates = []
        do {
            subtitleCandidates = try await client.searchMovieSubtitles(number: num)
            subtitleSearchDone = true
        } catch {
            subtitleError = error.localizedDescription
        }
        subtitleSearching = false
    }

    private func downloadSubtitle(_ cand: SubtitleSearchCandidate, for num: String) async {
        guard let client = appModel.apiClient, subtitleDownloadingId == nil else { return }
        subtitleDownloadingId = cand.id
        subtitleError = nil
        do {
            let saved = try await client.downloadMovieSubtitle(
                number: num,
                name: cand.title,
                lang: cand.lang,
                downloadUrl: cand.downloadUrl
            )
            savedSubtitles.removeAll(where: { $0.id == saved.id })
            savedSubtitles.insert(saved, at: 0)
        } catch {
            subtitleError = error.localizedDescription
        }
        subtitleDownloadingId = nil
    }

    private func deleteSubtitle(_ sub: SavedMovieSubtitle) async {
        guard let client = appModel.apiClient, subtitleDeletingId == nil else { return }
        subtitleDeletingId = sub.id
        subtitleError = nil
        do {
            try await client.deleteMovieSubtitle(id: sub.id)
            savedSubtitles.removeAll(where: { $0.id == sub.id })
        } catch {
            subtitleError = error.localizedDescription
        }
        subtitleDeletingId = nil
    }

    @ViewBuilder
    private func subtitleSection(number: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label("中文字幕（简体 / 繁体）", systemImage: "captions.bubble.fill")
                    .font(.footnote.bold())
                    .foregroundStyle(.purple)
                if !savedSubtitles.isEmpty {
                    Text("已绑定 \(savedSubtitles.count)")
                        .font(.caption2.bold())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.green.opacity(0.18), in: Capsule())
                        .foregroundStyle(.green)
                }
                Spacer()
                if subtitleSearchDone || subtitleError != nil {
                    Button("重新搜索") {
                        Task { await searchSubtitles(for: number) }
                    }
                    .font(.caption)
                    .disabled(subtitleSearching)
                }
            }

            ForEach(savedSubtitles) { sub in
                HStack(spacing: 8) {
                    Text(subtitleLangLabel(sub.lang))
                        .font(.caption2.bold())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background((sub.lang == "zh-CN" ? Color.blue : Color.orange).opacity(0.2), in: Capsule())
                        .foregroundStyle(sub.lang == "zh-CN" ? .blue : .orange)
                    Text(sub.name)
                        .font(.caption)
                        .lineLimit(1)
                    Spacer()
                    Text("自动挂载")
                        .font(.caption2)
                        .foregroundStyle(.green)
                    Button(role: .destructive) {
                        Task { await deleteSubtitle(sub) }
                    } label: {
                        if subtitleDeletingId == sub.id {
                            ProgressView().scaleEffect(0.7)
                        } else {
                            Image(systemName: "trash")
                                .font(.caption)
                        }
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color.green.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            }

            if let err = subtitleError {
                Text(err)
                    .font(.caption)
                    .foregroundStyle(.red)
            }

            if subtitleSearching {
                HStack(spacing: 8) {
                    ProgressView().scaleEffect(0.8)
                    Text("正在检索 \(number) 的简/繁体中文字幕...")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            } else if subtitleSearchDone {
                if subtitleCandidates.isEmpty {
                    Text("未在字幕源找到 \(number) 的简体/繁体中文字幕。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(subtitleCandidates) { cand in
                        HStack(spacing: 8) {
                            Text(cand.langLabel.isEmpty ? subtitleLangLabel(cand.lang) : cand.langLabel)
                                .font(.caption2.bold())
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background((cand.lang == "zh-CN" ? Color.blue : Color.orange).opacity(0.2), in: Capsule())
                                .foregroundStyle(cand.lang == "zh-CN" ? .blue : .orange)
                            Text(cand.title)
                                .font(.caption)
                                .lineLimit(1)
                            Spacer()
                            Button {
                                Task { await downloadSubtitle(cand, for: number) }
                            } label: {
                                if subtitleDownloadingId == cand.id {
                                    ProgressView().scaleEffect(0.75)
                                } else {
                                    Text("下载绑定")
                                        .font(.caption2.bold())
                                }
                            }
                            .buttonStyle(.bordered)
                            .controlSize(.mini)
                            .disabled(subtitleDownloadingId != nil)
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 7)
                        .background(Color.secondary.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
            }
        }
        .padding(12)
        .background(Color.purple.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
    }

    private func loadData(force: Bool = false) async {
        guard let client = appModel.apiClient else {
            errorMessage = "未连接到数据源"
            return
        }
        loading = true
        errorMessage = nil
        do {
            var meta: MovieMetadata?
            let requestedNumber = (currentNumber ?? number)?.trimmingCharacters(in: .whitespacesAndNewlines)
            if let num = requestedNumber, !num.isEmpty, !force {
                meta = try? await client.getMovieMetadata(number: num)
            }
            // Directory entries have no number yet. Resolve them on first open;
            // otherwise the sheet stays on an empty spinner until the user guesses
            // that the refresh button is required. Existing records still use the
            // cheap metadata lookup and never trigger a network scrape implicitly.
            if meta == nil, (force || requestedNumber == nil || requestedNumber?.isEmpty == true) {
                let sourceName = fileName?.trimmingCharacters(in: .whitespacesAndNewlines)
                guard assetId != nil || !(sourceName?.isEmpty ?? true) else {
                    metadata = nil
                    errorMessage = "缺少视频资产或文件名，无法刮削详情。"
                    loading = false
                    return
                }
                meta = try await client.scrapeAsset(
                    assetId: assetId ?? "",
                    fileName: sourceName ?? requestedNumber,
                    remotePath: remotePath,
                    number: requestedNumber,
                    force: force
                )
            }
            metadata = meta
            if let num = meta?.number {
                await loadAux(for: num)
            } else if !force {
                errorMessage = "暂无已验证元数据；点击刷新才会执行一次手动刮削。"
            }
        } catch {
            errorMessage = error.localizedDescription
        }
        loading = false
    }
}

private struct SampleGallerySelection: Identifiable {
    var id: Int { index }
    let index: Int
}

private struct SampleGalleryPagerView: View {
    @EnvironmentObject private var appModel: AppModel
    let images: [String]
    let onClose: () -> Void
    @State private var currentIndex: Int

    init(images: [String], initialIndex: Int, onClose: @escaping () -> Void) {
        self.images = images
        self.onClose = onClose
        _currentIndex = State(initialValue: min(max(0, initialIndex), max(0, images.count - 1)))
    }

    var body: some View {
        ZStack(alignment: .top) {
            Color.black.ignoresSafeArea()

            if !images.isEmpty {
                TabView(selection: $currentIndex) {
                    ForEach(Array(images.enumerated()), id: \.offset) { idx, raw in
                        Group {
                            if let url = appModel.proxyScraperImageURL(rawUrl: raw) {
                                CachedImageView(url: url, contentMode: .fit, minHeight: 360, maxPixel: 1800)
                                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                            } else {
                                Color.black
                            }
                        }
                        .tag(idx)
                    }
                }
                .tabViewStyle(.page(indexDisplayMode: .automatic))
                .ignoresSafeArea()
            }

            HStack {
                Text("\(min(currentIndex + 1, max(images.count, 1))) / \(images.count)")
                    .font(.subheadline.weight(.semibold))
                    .monospacedDigit()
                    .foregroundStyle(.white)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.ultraThinMaterial, in: Capsule())

                Spacer()

                Button {
                    onClose()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 15, weight: .bold))
                        .foregroundStyle(.white)
                        .frame(width: 34, height: 34)
                        .background(.ultraThinMaterial, in: Circle())
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
        }
    }
}
