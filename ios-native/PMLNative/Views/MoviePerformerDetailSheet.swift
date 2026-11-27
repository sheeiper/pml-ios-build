import SwiftUI
import PhotosUI
import UIKit

struct MoviePerformerDetailSheet: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var favs = EntityFavoritesStore.shared
    @Environment(\.dismiss) private var dismiss
    let performer: MoviePerformer
    var onBack: (() -> Void)? = nil
    var onFilterMovies: ((String) -> Void)? = nil
    var onPlay: ((String, String) -> Void)? = nil

    @State private var detail: MoviePerformerDetail?
    @State private var loading = false
    @State private var saving = false
    @State private var enriching = false
    @State private var errorMessage: String?
    private enum PerformerWorkTab: String, CaseIterable {
        case library = "库内作品"
        case recent = "近期新作"
    }

    private enum FullScreenModal: Identifiable {
        case player(PreviewItem)
        case avatar(MoviePerformer, URL?)

        var id: String {
            switch self {
            case .player(let item): return "player-\(item.id)"
            case .avatar(let p, _): return "avatar-\(p.id)"
            }
        }
    }

    private struct PendingPlayback {
        let assetId: String
        let title: String
    }

    @State private var detailTab: PerformerWorkTab = .library
    @State private var showEditor = false
    @State private var showMultiSourceSheet = false
    @State private var fullScreenModal: FullScreenModal?
    @State private var pendingPlayback: PendingPlayback?
    @State private var selectedMovie: MovieMetadata?
    @State private var recentResult: PerformerRecentReleasesResult?
    @State private var recentLoading = false
    @State private var recentError: String?
    @State private var includeOwnedRecent = false
    @State private var selectedReleaseForMagnets: PerformerRecentRelease?

    var body: some View {
        if onBack != nil {
            contentBody
        } else {
            NavigationStack {
                contentBody
            }
        }
    }

    private var contentBody: some View {
        mainScrollView
            .navigationTitle(detail?.person.name ?? performer.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbarContent }
            .task {
                await favs.ensureLoaded(client: model.apiClient)
                await load()
            }
            .sheet(isPresented: $showEditor) {
                MoviePerformerEditor(person: detail?.person ?? performer) { await save($0) }
                    .environmentObject(model)
            }
            .sheet(isPresented: $showMultiSourceSheet) {
                PerformerMultiSourceSheet(performerId: detail?.person.id ?? performer.id, performerName: detail?.person.name ?? performer.name) { updated in
                    detail = MoviePerformerDetail(person: updated, movies: detail?.movies ?? [], total: detail?.total ?? updated.movieCount)
                    showMultiSourceSheet = false
                }
                .environmentObject(model)
            }
            .sheet(item: $selectedReleaseForMagnets) { rel in
                OnlineMovieDetailSheet(
                    release: rel,
                    onPlayAsset: { assetId, title in
                        selectedReleaseForMagnets = nil
                        if let onPlay {
                            onPlay(assetId, title)
                        } else {
                            Task {
                                try? await Task.sleep(nanoseconds: 420_000_000)
                                await playAsset(assetId, title: title)
                            }
                        }
                    },
                    onFilterActor: { actorName in
                        selectedReleaseForMagnets = nil
                        onFilterMovies?(actorName)
                    }
                )
                .environmentObject(model)
            }
            .sheet(item: $selectedMovie) { movie in
                MovieDetailSheet(number: movie.number, assetId: movie.assetId, fileName: movie.title, remotePath: nil) { targetId in
                    selectedMovie = nil
                    if let onPlay {
                        onPlay(targetId, movie.title)
                    } else {
                        Task {
                            try? await Task.sleep(nanoseconds: 420_000_000)
                            await playAsset(targetId, title: movie.title)
                        }
                    }
                }
                .environmentObject(model)
            }
            .fullScreenCover(item: $fullScreenModal) { modal in
                fullScreenDestination(modal)
            }
    }

    private var mainScrollView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if loading && detail == nil {
                    ProgressView("加载演员档案…")
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 40)
                }
                if let errorMessage {
                    Label(errorMessage, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
                if let person = detail?.person ?? Optional(performer) {
                    personDetailSection(person)
                }
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private func personDetailSection(_ person: MoviePerformer) -> some View {
        profileHeader(person)
        actionsBar(person)
        factsGrid(person)
        if let bio = person.bio, !bio.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            Text(bio)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.2), in: RoundedRectangle(cornerRadius: 10))
        }
        worksSection(person)
    }

    @ViewBuilder
    private func actionsBar(_ person: MoviePerformer) -> some View {
        HStack(spacing: 8) {
            Button {
                showMultiSourceSheet = true
            } label: {
                Label("查询资料", systemImage: "magnifyingglass")
                    .font(.caption.weight(.semibold))
            }
            .buttonStyle(.borderedProminent)
            .tint(.purple)

            if let onFilterMovies {
                Button {
                    dismiss()
                    onFilterMovies(person.name)
                } label: {
                    Label("筛选影片", systemImage: "line.3.horizontal.decrease.circle")
                        .font(.caption.weight(.semibold))
                }
                .buttonStyle(.bordered)
            }

            Spacer()

            Button {
                showEditor = true
            } label: {
                Image(systemName: "pencil")
                    .font(.caption.weight(.semibold))
            }
            .buttonStyle(.bordered)
        }
    }

    @ViewBuilder
    private func factsGrid(_ person: MoviePerformer) -> some View {
        let facts = buildFacts(person)
        if !facts.isEmpty {
            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                ForEach(facts, id: \.label) { fact in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(fact.label)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        Text(fact.value)
                            .font(.caption.weight(.semibold))
                            .lineLimit(2)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
                    .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
                }
            }
        }
    }

    @ViewBuilder
    private func worksSection(_ person: MoviePerformer) -> some View {
        Picker("作品分区", selection: $detailTab) {
            Text("库内作品 (\(detail?.total ?? person.movieCount))").tag(PerformerWorkTab.library)
            Text(recentResult.map { "近期新作 (\($0.unownedCount))" } ?? "近期新作").tag(PerformerWorkTab.recent)
        }
        .pickerStyle(.segmented)
        .onChange(of: detailTab) { _, next in
            if next == .recent && recentResult == nil && !recentLoading {
                Task { await loadRecentReleases(forceRefresh: false) }
            }
        }

        if detailTab == .library {
            if let movies = detail?.movies, !movies.isEmpty {
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                    ForEach(movies) { movie in
                        movieCard(movie)
                    }
                }
            } else if !loading {
                Text("暂无关联作品").font(.footnote).foregroundStyle(.secondary)
            }
        } else {
            recentReleasesSection
        }
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        if let onBack {
            ToolbarItem(placement: .topBarLeading) {
                Button {
                    onBack()
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "chevron.left")
                        Text("演员列表")
                    }
                    .font(.body.weight(.medium))
                }
            }
        }
        ToolbarItemGroup(placement: .topBarTrailing) {
            let targetId = detail?.person.id ?? performer.id
            let isFav = favs.isPerformerFav(targetId)
            Button {
                Task { await favs.toggle(client: model.apiClient, type: .performer, id: targetId) }
            } label: {
                Image(systemName: isFav ? "heart.fill" : "heart")
                    .foregroundStyle(isFav ? .pink : .secondary)
            }
            if onBack == nil {
                Button("完成") { dismiss() }
            }
        }
    }

    private func handleMovieDismiss() {
        if let pending = pendingPlayback {
            pendingPlayback = nil
            if let onPlay {
                onPlay(pending.assetId, pending.title)
            } else {
                Task {
                    try? await Task.sleep(nanoseconds: 420_000_000)
                    await playAsset(pending.assetId, title: pending.title)
                }
            }
        }
    }

    @ViewBuilder
    private func fullScreenDestination(_ modal: FullScreenModal) -> some View {
        switch modal {
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
            .environmentObject(model)
        case .avatar(let person, let avatarUrl):
            PerformerAvatarViewer(person: person, avatarUrl: avatarUrl)
        }
    }

    @ViewBuilder
    private var recentReleasesSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label("近期新作（未入库）", systemImage: "sparkles")
                    .font(.subheadline.bold())
                    .foregroundStyle(.purple)
                if let res = recentResult {
                    Text("\(res.unownedCount)/\(res.totalFetched)")
                        .font(.caption2.bold())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.purple.opacity(0.18), in: Capsule())
                        .foregroundStyle(.purple)
                }
                Spacer()
                Button {
                    includeOwnedRecent.toggle()
                    Task { await loadRecentReleases(forceRefresh: false) }
                } label: {
                    Text(includeOwnedRecent ? "仅看未入库" : "含已入库")
                        .font(.caption2)
                }
                .buttonStyle(.bordered)
                .controlSize(.mini)

                Button {
                    Task { await loadRecentReleases(forceRefresh: true) }
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.caption)
                }
                .buttonStyle(.bordered)
                .controlSize(.mini)
                .disabled(recentLoading)
            }

            if let err = recentError {
                Text(err).font(.caption).foregroundStyle(.red)
            } else if recentLoading && recentResult == nil {
                HStack(spacing: 8) {
                    ProgressView().scaleEffect(0.8)
                    Text("正在查询近期新作并比对本地库…").font(.caption).foregroundStyle(.secondary)
                }
            } else if let items = recentResult?.items, !items.isEmpty {
                LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                    ForEach(items) { rel in
                        Button {
                            selectedReleaseForMagnets = rel
                        } label: {
                            VStack(alignment: .leading, spacing: 5) {
                                ZStack {
                                    Color.black.opacity(0.85)
                                    CachedImageView(
                                        url: rel.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) },
                                        contentMode: .fit,
                                        minHeight: 96,
                                        maxPixel: 540
                                    )
                                }
                                .overlay(alignment: .topLeading) {
                                    HStack {
                                        Text(rel.number)
                                            .font(.system(size: 9, weight: .bold))
                                            .padding(.horizontal, 5)
                                            .padding(.vertical, 2)
                                            .background(.black.opacity(0.72), in: RoundedRectangle(cornerRadius: 4))
                                            .foregroundStyle(.white)
                                        Spacer()
                                        Text(rel.inLibrary ? "已入库" : "详情")
                                            .font(.system(size: 9, weight: .bold))
                                            .padding(.horizontal, 5)
                                            .padding(.vertical, 2)
                                            .background((rel.inLibrary ? Color.green : Color.purple).opacity(0.85), in: RoundedRectangle(cornerRadius: 4))
                                            .foregroundStyle(.white)
                                    }
                                    .padding(5)
                                }
                                .aspectRatio(3 / 2, contentMode: .fit)
                                .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))

                                Text(rel.title)
                                    .font(.caption.weight(.semibold))
                                    .lineLimit(2)
                                    .foregroundStyle(.primary)
                                if let d = rel.releaseDate, !d.isEmpty {
                                    let isFuture = d > ISO8601DateFormatter().string(from: Date()).prefix(10)
                                    HStack(spacing: 3) {
                                        if isFuture {
                                            Text("预定")
                                                .font(.system(size: 8, weight: .bold))
                                                .padding(.horizontal, 4)
                                                .padding(.vertical, 1)
                                                .background(Color.orange.opacity(0.25), in: RoundedRectangle(cornerRadius: 3))
                                                .foregroundStyle(.orange)
                                        }
                                        Text(d)
                                            .font(.caption2)
                                            .foregroundStyle(isFuture ? .orange : .secondary)
                                    }
                                }
                            }
                            .opacity(rel.inLibrary ? 0.65 : 1.0)
                        }
                        .buttonStyle(.plain)
                    }
                }
            } else if let res = recentResult {
                Text(res.totalFetched > 0 ? "近期发布的 \(res.totalFetched) 部作品已全部入库！" : "暂未查到该演员的近期线上作品。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .background(Color.purple.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
    }

    private func loadRecentReleases(forceRefresh: Bool) async {
        guard let client = model.apiClient else { return }
        recentLoading = true
        recentError = nil
        do {
            recentResult = try await client.getPerformerRecentReleases(
                id: performer.id,
                includeOwned: includeOwnedRecent,
                forceRefresh: forceRefresh
            )
        } catch {
            recentError = error.localizedDescription
        }
        recentLoading = false
    }

    private func profileHeader(_ person: MoviePerformer) -> some View {
        HStack(spacing: 14) {
            Button {
                if let avatar = person.avatarUrl, !avatar.isEmpty {
                    fullScreenModal = .avatar(person, avatarURL(avatar))
                }
            } label: {
                ZStack(alignment: .bottomTrailing) {
                    PerformerAvatarView(value: person.avatarUrl, proxy: avatarURL(person.avatarUrl), fallbackName: person.name)
                        .frame(width: 82, height: 82)
                        .clipShape(Circle())
                        .overlay(Circle().stroke(Color.purple.opacity(0.4), lineWidth: 2))

                    if let avatar = person.avatarUrl, !avatar.isEmpty {
                        Image(systemName: "magnifyingglass")
                            .font(.system(size: 11, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(4)
                            .background(Circle().fill(Color.purple))
                            .offset(x: 2, y: 2)
                    }
                }
            }
            .buttonStyle(.plain)
            .disabled((person.avatarUrl ?? "").isEmpty)

            VStack(alignment: .leading, spacing: 5) {
                Text(person.name).font(.title2.bold())
                if !person.aliases.isEmpty {
                    Text(person.aliases.prefix(4).joined(separator: " / "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Text("\(detail?.total ?? person.movieCount) 部库内作品")
                    .font(.caption)
                    .foregroundStyle(.purple)

                if person.twitterUrl != nil || person.instagramUrl != nil {
                    HStack(spacing: 8) {
                        if let raw = person.twitterUrl, let url = URL(string: raw), !raw.isEmpty {
                            Link(destination: url) {
                                Label("Twitter/X", systemImage: "link")
                                    .font(.caption2.weight(.semibold))
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(Color.blue.opacity(0.16), in: Capsule())
                            }
                        }
                        if let raw = person.instagramUrl, let url = URL(string: raw), !raw.isEmpty {
                            Link(destination: url) {
                                Label("Instagram", systemImage: "camera")
                                    .font(.caption2.weight(.semibold))
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(Color.pink.opacity(0.16), in: Capsule())
                            }
                        }
                    }
                    .padding(.top, 2)
                }
            }
            Spacer()
        }
        .padding(14)
        .background(.quaternary.opacity(0.22), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func movieCard(_ movie: MovieMetadata) -> some View {
        Button { selectedMovie = movie } label: {
            VStack(alignment: .leading, spacing: 5) {
                ZStack {
                    Color.black.opacity(0.85)
                    CachedImageView(url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fit, minHeight: 96, maxPixel: 600)
                }
                .overlay(alignment: .topLeading) {
                    Text(movie.number)
                        .font(.system(size: 9, weight: .bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.black.opacity(0.72), in: RoundedRectangle(cornerRadius: 4))
                        .foregroundStyle(.white)
                        .padding(5)
                }
                .aspectRatio(3 / 2, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))

                Text(movie.translatedTitle?.isEmpty == false ? movie.translatedTitle! : movie.title)
                    .font(.caption.weight(.semibold))
                    .lineLimit(2)
                    .foregroundStyle(.primary)
                if let date = movie.releaseDate, !date.isEmpty {
                    Text(date)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .buttonStyle(.plain)
    }

    private struct FactItem { let label: String; let value: String }

    private func buildFacts(_ p: MoviePerformer) -> [FactItem] {
        var list: [FactItem] = []
        if let birth = p.birthDate, !birth.isEmpty {
            list.append(FactItem(label: "生日", value: formatBirth(birth)))
        }
        if let h = p.heightCm, h > 0 {
            list.append(FactItem(label: "身高", value: "\(h) cm"))
        }
        if let m = p.measurements, !m.isEmpty {
            list.append(FactItem(label: "身材", value: m))
        }
        if let bt = p.bloodType, !bt.isEmpty {
            list.append(FactItem(label: "血型", value: bt))
        }
        if let home = p.hometown, !home.isEmpty {
            list.append(FactItem(label: "出生地", value: home))
        }
        if let debut = p.debutYear, !debut.isEmpty {
            list.append(FactItem(label: "出道 / 活跃", value: debut))
        }
        if let agency = p.agency, !agency.isEmpty {
            list.append(FactItem(label: "所属事务所", value: agency))
        }
        if let hobby = p.hobby, !hobby.isEmpty {
            list.append(FactItem(label: "爱好 / 特长", value: hobby))
        }
        return list
    }

    private func formatBirth(_ raw: String) -> String {
        let parts = raw.split(whereSeparator: { $0 == "-" || $0 == "/" }).compactMap { Int($0) }
        guard parts.count >= 3 else { return raw }
        let (y, m, d) = (parts[0], parts[1], parts[2])
        let now = Calendar.current.dateComponents([.year, .month, .day], from: Date())
        var age = (now.year ?? y) - y
        if (now.month ?? 1) < m || ((now.month ?? 1) == m && (now.day ?? 1) < d) { age -= 1 }
        let signs: [(Int, Int, String)] = [
            (1, 20, "水瓶座"), (2, 19, "双鱼座"), (3, 21, "白羊座"), (4, 20, "金牛座"),
            (5, 21, "双子座"), (6, 22, "巨蟹座"), (7, 23, "狮子座"), (8, 23, "处女座"),
            (9, 23, "天秤座"), (10, 24, "天蝎座"), (11, 23, "射手座"), (12, 22, "摩羯座"),
        ]
        var zodiac = "摩羯座"
        for (sm, sd, name) in signs.reversed() {
            if m > sm || (m == sm && d >= sd) { zodiac = name; break }
        }
        return (18...90).contains(age) ? "\(raw)（\(age)岁 · \(zodiac)）" : "\(raw)（\(zodiac)）"
    }

    private func avatarURL(_ value: String?) -> URL? {
        guard let value, !value.isEmpty else { return nil }
        if value.hasPrefix("data:") { return URL(string: value) }
        return model.proxyScraperImageURL(rawUrl: value)
    }

    private func load() async {
        guard let client = model.apiClient else { errorMessage = "未连接到数据源"; return }
        loading = true; defer { loading = false }
        do { detail = try await client.getMoviePerformer(id: performer.id) }
        catch { errorMessage = error.localizedDescription }
    }

    private func enrich(force: Bool) async {
        guard let client = model.apiClient else { return }
        enriching = true; errorMessage = nil; defer { enriching = false }
        do {
            let updated = try await client.enrichMoviePerformer(id: performer.id, force: force)
            detail = MoviePerformerDetail(person: updated, movies: detail?.movies ?? [], total: detail?.total ?? updated.movieCount)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func save(_ input: MoviePerformerEditor.Input) async {
        guard let client = model.apiClient else { return }
        saving = true; defer { saving = false }
        do {
            let person = try await client.updateMoviePerformer(id: performer.id, name: input.name, aliases: input.aliases, avatarURL: input.avatarURL, profileURL: input.profileURL)
            detail = MoviePerformerDetail(person: person, movies: detail?.movies ?? [], total: detail?.total ?? person.movieCount)
            showEditor = false
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func playAsset(_ assetId: String, title: String) async {
        guard let source = await model.playbackSource(assetId: assetId, remotePath: nil) else {
            errorMessage = model.playbackError ?? "无法解析视频直链"
            return
        }
        fullScreenModal = .player(PreviewItem(url: source.url, isVideo: true, assetId: assetId, title: title, fallbackURL: source.fallbackURL, sourceLabel: source.label, remotePath: nil))
    }
}

struct MovieMagnetsSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let release: PerformerRecentRelease

    @State private var magnets: [MovieMagnetItem] = []
    @State private var loading = false
    @State private var errorMessage: String?
    @State private var copiedMagnet: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if let raw = release.coverUrl, let url = model.proxyScraperImageURL(rawUrl: raw) {
                        ZStack {
                            Color.black
                            CachedImageView(url: url, contentMode: .fit, minHeight: 180, maxPixel: 1000)
                        }
                        .aspectRatio(3 / 2, contentMode: .fit)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }

                    VStack(alignment: .leading, spacing: 6) {
                        HStack(spacing: 8) {
                            Text(release.number)
                                .font(.caption.bold())
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(LinearGradient(colors: [.purple, .pink], startPoint: .leading, endPoint: .trailing))
                                .foregroundStyle(.white)
                                .clipShape(RoundedRectangle(cornerRadius: 5))
                            if let d = release.releaseDate, !d.isEmpty {
                                Text(d).font(.caption).foregroundStyle(.secondary)
                            }
                            if release.inLibrary {
                                Text("库内已有")
                                    .font(.caption2.bold())
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                                    .background(Color.green.opacity(0.18), in: Capsule())
                                    .foregroundStyle(.green)
                            }
                        }
                        Text(release.title).font(.subheadline.bold())
                    }

                    if let err = errorMessage {
                        Text(err).font(.caption).foregroundStyle(.red)
                    } else if loading {
                        HStack(spacing: 8) {
                            ProgressView()
                            Text("正在聚合 JavDB / Sukebei 磁力链接…").font(.caption).foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 24)
                    } else if magnets.isEmpty {
                        Text("暂无可用磁力链接（可能尚未发售或暂无公开种子）。")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .padding(.vertical, 16)
                    } else {
                        ForEach(magnets) { item in
                            VStack(alignment: .leading, spacing: 6) {
                                Text(item.title)
                                    .font(.caption.weight(.semibold))
                                    .lineLimit(3)
                                HStack(spacing: 6) {
                                    ForEach(item.tags, id: \.self) { tag in
                                        Text(tag)
                                            .font(.caption2.bold())
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background((tag.contains("字幕") ? Color.orange : Color.blue).opacity(0.2), in: Capsule())
                                            .foregroundStyle(tag.contains("字幕") ? .orange : .blue)
                                    }
                                    if let size = item.size { Text(size).font(.caption2).foregroundStyle(.secondary) }
                                    if let seed = item.seeders { Text("做种 \(seed)").font(.caption2).foregroundStyle(.secondary) }
                                    Spacer()
                                    Text(item.source).font(.caption2).foregroundStyle(.purple)
                                }
                                Button {
                                    UIPasteboard.general.string = item.magnet
                                    copiedMagnet = item.magnet
                                    UINotificationFeedbackGenerator().notificationOccurred(.success)
                                } label: {
                                    Label(copiedMagnet == item.magnet ? "已复制磁力链接" : "复制磁力链接", systemImage: copiedMagnet == item.magnet ? "checkmark" : "doc.on.doc")
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
                .padding(16)
            }
            .navigationTitle("\(release.number) 磁力")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        Task { await loadMagnets(forceRefresh: true) }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(loading)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
            .task { await loadMagnets(forceRefresh: false) }
        }
    }

    private func loadMagnets(forceRefresh: Bool) async {
        guard let client = model.apiClient else { return }
        loading = true
        errorMessage = nil
        do {
            magnets = try await client.getMovieMagnets(number: release.number, detailUrl: release.detailUrl, forceRefresh: forceRefresh)
        } catch {
            errorMessage = error.localizedDescription
        }
        loading = false
    }
}

struct PerformerAvatarView: View {
    let value: String?
    let proxy: URL?
    let fallbackName: String
    var body: some View {
        if let value, value.hasPrefix("data:image/"), let comma = value.firstIndex(of: ","),
           let data = Data(base64Encoded: String(value[value.index(after: comma)...])), let image = UIImage(data: data) {
            Image(uiImage: image).resizable().scaledToFill()
        } else if let proxy {
            CachedImageView(url: proxy, contentMode: .fill, minHeight: 76, maxPixel: 360)
        } else {
            ZStack {
                LinearGradient(colors: [.indigo, .purple], startPoint: .topLeading, endPoint: .bottomTrailing)
                Text(String(fallbackName.prefix(1)))
                    .font(.title2.bold())
                    .foregroundStyle(.white)
            }
        }
    }
}

struct PerformerAvatarViewer: View {
    let person: MoviePerformer
    let avatarUrl: URL?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.ignoresSafeArea()

            VStack(spacing: 16) {
                Spacer()
                if let raw = person.avatarUrl, raw.hasPrefix("data:image/"),
                   let comma = raw.firstIndex(of: ","),
                   let data = Data(base64Encoded: String(raw[raw.index(after: comma)...])),
                   let image = UIImage(data: data) {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                        .padding(20)
                } else if let avatarUrl {
                    CachedImageView(url: avatarUrl, contentMode: .fit, minHeight: 240, maxPixel: 2048)
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                        .padding(20)
                } else {
                    Text("暂无头像图片")
                        .foregroundStyle(.secondary)
                }

                VStack(spacing: 4) {
                    Text(person.name)
                        .font(.title3.bold())
                        .foregroundStyle(.white)
                    if !person.aliases.isEmpty {
                        Text(person.aliases.joined(separator: " / "))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Rectangle())
            .onTapGesture { dismiss() }

            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 28))
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.white.opacity(0.85))
                    .padding(20)
            }
        }
    }
}

private struct MoviePerformerEditor: View {
    struct Input { let name: String; let aliases: [String]; let avatarURL: String?; let profileURL: String? }
    @Environment(\.dismiss) private var dismiss
    let person: MoviePerformer
    let onSave: (Input) async -> Void
    @State private var name: String
    @State private var aliases: String
    @State private var avatarURL: String
    @State private var profileURL: String
    @State private var avatarItem: PhotosPickerItem?
    @State private var notice = ""

    init(person: MoviePerformer, onSave: @escaping (Input) async -> Void) {
        self.person = person; self.onSave = onSave
        _name = State(initialValue: person.name); _aliases = State(initialValue: person.aliases.joined(separator: ", "))
        _avatarURL = State(initialValue: person.avatarUrl ?? ""); _profileURL = State(initialValue: person.profileUrl ?? "")
    }
    var body: some View {
        NavigationStack {
            Form {
                Section("显示资料") { TextField("姓名", text: $name); TextField("别名（逗号分隔）", text: $aliases, axis: .vertical); TextField("头像 URL（可留空）", text: $avatarURL); TextField("公开资料 URL（可留空）", text: $profileURL) }
                Section("手动头像") {
                    PhotosPicker(selection: $avatarItem, matching: .images) { Label("从照片选择并上传", systemImage: "photo.on.rectangle") }
                    Text("服务端限制 JPEG/PNG/WebP，最大 2 MiB。").font(.caption2).foregroundStyle(.secondary)
                    if !notice.isEmpty { Text(notice).font(.caption).foregroundStyle(.secondary) }
                }
                Button("保存") { Task { await save() } }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.navigationTitle("编辑演员资料").navigationBarTitleDisplayMode(.inline).toolbar { ToolbarItem(placement: .topBarLeading) { Button("取消") { dismiss() } } }
            .onChange(of: avatarItem) { _, item in guard let item else { return }; Task { await importAvatar(item) } }
        }
    }
    private func importAvatar(_ item: PhotosPickerItem) async {
        guard let data = try? await item.loadTransferable(type: Data.self), let b64 = pmlJPEGBase64(data, maxDimension: 900, quality: 0.82) else { notice = "读取头像失败"; return }
        avatarURL = "data:image/jpeg;base64,\(b64)"; notice = "已载入本地头像，保存后上传"
    }
    private func save() async {
        let cleanAliases = aliases.split(whereSeparator: { "，,;；/".contains($0) }).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        await onSave(Input(name: name.trimmingCharacters(in: .whitespacesAndNewlines), aliases: cleanAliases, avatarURL: avatarURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : avatarURL.trimmingCharacters(in: .whitespacesAndNewlines), profileURL: profileURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : profileURL.trimmingCharacters(in: .whitespacesAndNewlines)))
    }
}
