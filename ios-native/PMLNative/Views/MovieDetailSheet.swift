import SwiftUI

public struct MovieDetailSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var appModel: AppModel

    public let number: String?
    public let assetId: String?
    public let fileName: String?
    public let remotePath: String?
    public let onPlay: ((String) -> Void)?

    @State private var loading = false
    @State private var errorMessage: String?
    @State private var metadata: MovieMetadata?
    @State private var relatedMovies: [MovieMetadata] = []
    @State private var selectedSampleURL: String?
    @State private var currentNumber: String?

    public init(
        number: String? = nil,
        assetId: String? = nil,
        fileName: String? = nil,
        remotePath: String? = nil,
        onPlay: ((String) -> Void)? = nil
    ) {
        self.number = number
        self.assetId = assetId
        self.fileName = fileName
        self.remotePath = remotePath
        self.onPlay = onPlay
    }

    public var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if loading {
                        HStack {
                            Spacer()
                            VStack(spacing: 12) {
                                ProgressView()
                                    .scaleEffect(1.2)
                                Text("正在联网刮削日本影视百科与剧照...")
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
                        // Hero section
                        HStack(alignment: .top, spacing: 16) {
                            CachedImageView(url: meta.coverUrl.flatMap { appModel.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fill, minHeight: 90, maxPixel: 900)
                                .aspectRatio(16 / 9, contentMode: .fit)
                                .frame(width: 160, height: 90)
                                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                                .shadow(color: .black.opacity(0.3), radius: 6, x: 0, y: 3)

                            VStack(alignment: .leading, spacing: 8) {
                                HStack(spacing: 6) {
                                    Text(meta.number)
                                        .font(.caption.bold())
                                        .padding(.horizontal, 7)
                                        .padding(.vertical, 3)
                                        .background(LinearGradient(colors: [.purple, .indigo], startPoint: .leading, endPoint: .trailing))
                                        .foregroundStyle(.white)
                                        .clipShape(RoundedRectangle(cornerRadius: 5))

                                    if let date = meta.releaseDate {
                                        Text(date)
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }

                                    if let duration = meta.durationMinutes, duration > 0 {
                                        Text("\(duration) 分钟")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }
                                }

                                Text(meta.title)
                                    .font(.subheadline.bold())
                                    .lineLimit(4)
                                    .foregroundStyle(.primary)

                                if let orig = meta.originalTitle, !orig.isEmpty, orig != meta.title {
                                    Text(orig)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(2)
                                }

                                if let maker = meta.maker {
                                    HStack(spacing: 4) {
                                        Text("片商:").font(.caption2).foregroundStyle(.secondary)
                                        Text(maker).font(.caption.bold()).foregroundStyle(.primary)
                                    }
                                }

                                if let series = meta.series {
                                    HStack(spacing: 4) {
                                        Text("系列:").font(.caption2).foregroundStyle(.secondary)
                                        Text(series).font(.caption).foregroundStyle(.secondary)
                                    }
                                }

                                if let director = meta.director {
                                    HStack(spacing: 4) {
                                        Text("导演:").font(.caption2).foregroundStyle(.secondary)
                                        Text(director).font(.caption).foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }

                        // Play & Rescrape Buttons
                        HStack(spacing: 12) {
                            Button {
                                if let targetId = meta.assetId ?? assetId {
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

                        // Actresses
                        if !meta.actors.isEmpty {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(spacing: 4) {
                                    Image(systemName: "heart.fill")
                                        .font(.caption2)
                                        .foregroundStyle(.pink)
                                    Text("出演女优")
                                        .font(.footnote.bold())
                                        .foregroundStyle(.secondary)
                                }

                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 8) {
                                        ForEach(meta.actors, id: \.self) { actress in
                                            Label(actress, systemImage: "heart.fill")
                                                .font(.caption.weight(.semibold))
                                                .padding(.horizontal, 10)
                                                .padding(.vertical, 5)
                                                .background(Color.pink.opacity(0.14))
                                                .foregroundStyle(Color.pink)
                                                .clipShape(Capsule())
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
                                            Text(tag)
                                                .font(.caption2)
                                                .padding(.horizontal, 8)
                                                .padding(.vertical, 4)
                                                .background(Color.secondary.opacity(0.12))
                                                .foregroundStyle(.secondary)
                                                .clipShape(RoundedRectangle(cornerRadius: 4))
                                        }
                                    }
                                }
                            }
                        }

                        // Sample Images
                        if !meta.sampleImages.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("官方高清剧照 (\(meta.sampleImages.count))")
                                    .font(.footnote.bold())
                                    .foregroundStyle(.secondary)

                                ScrollView(.horizontal, showsIndicators: false) {
                                    LazyHStack(spacing: 8) {
                                        ForEach(meta.sampleImages, id: \.self) { sample in
                                            if let url = appModel.proxyScraperImageURL(rawUrl: sample) {
                                                Button {
                                                    selectedSampleURL = sample
                                                } label: {
                                                    CachedImageView(url: url, contentMode: .fill, minHeight: 95, maxPixel: 480)
                                                        .frame(width: 140, height: 95)
                                                    .clipShape(RoundedRectangle(cornerRadius: 6))
                                                }
                                                .buttonStyle(.plain)
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Related movies
                        if !relatedMovies.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("库内关联推荐 (同女优 / 同厂牌)")
                                    .font(.footnote.bold())
                                    .foregroundStyle(.secondary)

                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 10) {
                                        ForEach(relatedMovies) { rel in
                                            Button {
                                                metadata = rel
                                                currentNumber = rel.number
                                                Task {
                                                    relatedMovies = (try? await appModel.apiClient?.getRelatedMovies(number: rel.number, limit: 8)) ?? []
                                                }
                                            } label: {
                                                VStack(alignment: .leading, spacing: 4) {
                                                    if let cover = rel.coverUrl, let url = appModel.proxyScraperImageURL(rawUrl: cover) {
                                                        CachedImageView(url: url, contentMode: .fill, minHeight: 75, maxPixel: 360)
                                                            .frame(width: 110, height: 75)
                                                        .clipShape(RoundedRectangle(cornerRadius: 6))
                                                    }
                                                    Text(rel.number)
                                                        .font(.caption2.bold())
                                                        .foregroundStyle(.primary)
                                                    Text(rel.title)
                                                        .font(.system(size: 10))
                                                        .foregroundStyle(.secondary)
                                                        .lineLimit(1)
                                                        .frame(width: 110, alignment: .leading)
                                                }
                                            }
                                            .buttonStyle(.plain)
                                        }
                                    }
                                }
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

    private func loadData(force: Bool = false) async {
        guard let client = appModel.apiClient else {
            errorMessage = "未连接到数据源"
            return
        }
        loading = true
        errorMessage = nil
        do {
            var meta: MovieMetadata?
            let requestedNumber = currentNumber ?? number
            if let num = requestedNumber, !force {
                meta = try? await client.getMovieMetadata(number: num)
            }
            if meta == nil, force {
                meta = try await client.scrapeAsset(
                    assetId: assetId ?? "",
                    fileName: fileName ?? requestedNumber,
                    remotePath: remotePath,
                    number: requestedNumber,
                    force: true
                )
            }
            metadata = meta
            if let num = meta?.number {
                relatedMovies = (try? await client.getRelatedMovies(number: num, limit: 8)) ?? []
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
