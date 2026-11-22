import SwiftUI

public struct MovieDetailSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var appModel: AppModel

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
    @State private var selectedSampleURL: String?
    @State private var selectedPerformer: MoviePerformer?
    @State private var currentNumber: String?

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
                                Text(meta.number)
                                    .font(.caption.bold())
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 3)
                                    .background(LinearGradient(colors: [.purple, .pink], startPoint: .leading, endPoint: .trailing))
                                    .foregroundStyle(.white)
                                    .clipShape(RoundedRectangle(cornerRadius: 5))

                                if let date = meta.releaseDate, !date.isEmpty {
                                    Text(date)
                                        .font(.caption)
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

                        // Play & Rescrape Buttons
                        HStack(spacing: 12) {
                            Button {
                                if let targetId = parts.first?.assetId ?? meta.assetId ?? assetId {
                                    dismiss()
                                    onPlay?(targetId)
                                }
                            } label: {
                                Label("立即播放", systemImage: "play.fill")
                                    .font(.headline)
                                    .frame(maxWidth: .infinity)
                                    .padding(.vertical, 8)
                            }
                            .buttonStyle(.borderedProminent)
                            .tint(.purple)

                            Button {
                                Task { await loadData(force: true) }
                            } label: {
                                Image(systemName: "arrow.clockwise")
                                    .font(.headline)
                                    .padding(.vertical, 8)
                                    .padding(.horizontal, 14)
                            }
                            .buttonStyle(.bordered)
                        }

                        // Multi-part / local file switcher
                        if !parts.isEmpty {
                            VStack(alignment: .leading, spacing: 6) {
                                Text(parts.count > 1 ? "多分片视频（共 \(parts.count) 个文件）" : "关联本地视频")
                                    .font(.footnote.bold())
                                    .foregroundStyle(.secondary)
                                ForEach(Array(parts.enumerated()), id: \.element.id) { idx, part in
                                    Button {
                                        dismiss()
                                        onPlay?(part.assetId)
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
                                            VStack(alignment: .leading, spacing: 2) {
                                                Text(part.fileName)
                                                    .font(.caption.weight(.semibold))
                                                    .lineLimit(1)
                                                    .foregroundStyle(.primary)
                                                Text(part.remotePath)
                                                    .font(.system(size: 10))
                                                    .lineLimit(1)
                                                    .foregroundStyle(.secondary)
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
                                            Button {
                                                Task { await openPerformer(named: actress) }
                                            } label: {
                                                Label(actress, systemImage: "person.circle.fill")
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
                                        ForEach(meta.sampleImages, id: \.self) { sample in
                                            if let url = appModel.proxyScraperImageURL(rawUrl: sample) {
                                                Button {
                                                    selectedSampleURL = sample
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
            .navigationTitle(metadata?.number ?? "影视百科详情")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
            .sheet(item: $selectedPerformer) { person in
                MoviePerformerDetailSheet(
                    performer: person,
                    onFilterMovies: onFilterActor.map { handler in { actorName in dismiss(); handler(actorName) } },
                    onPlay: { targetId, _ in
                        dismiss()
                        onPlay?(targetId)
                    }
                )
                .environmentObject(appModel)
            }
            .sheet(item: Binding<SampleImageItem?>(
                get: { selectedSampleURL.map { SampleImageItem(url: $0) } },
                set: { selectedSampleURL = $0?.url }
            )) { item in
                if let u = appModel.proxyScraperImageURL(rawUrl: item.url) {
                    NavigationStack {
                        ZStack {
                            Color.black.ignoresSafeArea()
                            CachedImageView(url: u, contentMode: .fit, minHeight: 360, maxPixel: 1800)
                        }
                        .toolbar {
                            ToolbarItem(placement: .topBarTrailing) {
                                Button("关闭") { selectedSampleURL = nil }
                                    .foregroundStyle(.white)
                            }
                        }
                    }
                }
            }
            .task {
                currentNumber = number
                await loadData()
            }
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

    private struct SpecItem { let label: String; let value: String; var isMaker: Bool = false }

    private func buildSpecs(_ meta: MovieMetadata) -> [SpecItem] {
        var list: [SpecItem] = []
        if let maker = meta.maker, !maker.isEmpty { list.append(SpecItem(label: "片商", value: maker, isMaker: true)) }
        if let label = meta.label, !label.isEmpty { list.append(SpecItem(label: "发行厂牌", value: label)) }
        if let series = meta.series, !series.isEmpty { list.append(SpecItem(label: "所属系列", value: series)) }
        if let director = meta.director, !director.isEmpty { list.append(SpecItem(label: "导演", value: director)) }
        list.append(SpecItem(label: "数据来源", value: meta.source.isEmpty ? "未知" : meta.source))
        return list
    }

    private func formatBytes(_ bytes: Int64) -> String {
        let gb = Double(bytes) / 1_073_741_824.0
        if gb >= 1.0 { return String(format: "%.2f GB", gb) }
        return "\(bytes / 1_048_576) MB"
    }

    private func openPerformer(named name: String) async {
        guard let client = appModel.apiClient else { return }
        if let list = try? await client.listMoviePerformers(query: name, limit: 10, offset: 0) {
            let exact = list.items.first(where: { $0.name == name || $0.aliases.contains(name) })
            selectedPerformer = exact ?? list.items.first
        }
    }

    private func loadAux(for num: String) async {
        guard let client = appModel.apiClient else { return }
        async let fetchedParts = try? client.listMovieAssets(number: num)
        async let fetchedRelated = try? client.getRelatedMoviesGrouped(number: num, limitPerGroup: 8)
        parts = (await fetchedParts) ?? []
        relatedGrouped = await fetchedRelated
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

private struct SampleImageItem: Identifiable {
    var id: String { url }
    let url: String
}
