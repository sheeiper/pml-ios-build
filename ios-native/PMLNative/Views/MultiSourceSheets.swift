import SwiftUI

/// 作品详情多源对比与切换 Sheet（JavDB / JavBus / Jav321 / AV-Wiki / FC2）
struct MovieMultiSourceSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let number: String
    let currentSource: String?
    let onAdopted: (MovieMetadata) -> Void

    @State private var candidates: [MovieMetadata] = []
    @State private var loading = true
    @State private var applyingSource: String?
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            Group {
                if loading {
                    ProgressView("正在并发查询 JavDB / JavBus / Jav321 / AV-Wiki / JavFree…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let errorMessage {
                    ContentUnavailableView("查询失败", systemImage: "exclamationmark.triangle", description: Text(errorMessage))
                } else if candidates.isEmpty {
                    ContentUnavailableView("未查到多源资料", systemImage: "magnifyingglass", description: Text("请稍后重试或检查番号拼写。"))
                } else {
                    ScrollView {
                        LazyVStack(spacing: 14) {
                            ForEach(candidates, id: \.source) { cand in
                                VStack(alignment: .leading, spacing: 10) {
                                    ZStack {
                                        Color.black.opacity(0.88)
                                        if let raw = cand.coverUrl, let url = model.proxyScraperImageURL(rawUrl: raw) {
                                            CachedImageView(url: url, contentMode: .fit, minHeight: 150, maxPixel: 800)
                                        }
                                    }
                                    .overlay(alignment: .topLeading) {
                                        Text(cand.source.uppercased())
                                            .font(.caption2.bold())
                                            .padding(.horizontal, 8)
                                            .padding(.vertical, 3)
                                            .background(Color.purple.opacity(0.9), in: RoundedRectangle(cornerRadius: 6))
                                            .foregroundStyle(.white)
                                            .padding(8)
                                    }
                                    .aspectRatio(3 / 2, contentMode: .fit)
                                    .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

                                    Text(cand.title)
                                        .font(.subheadline.weight(.semibold))
                                        .lineLimit(3)

                                    if let orig = cand.originalTitle, !orig.isEmpty, orig != cand.title {
                                        Text(orig)
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                            .lineLimit(2)
                                    }

                                    HStack(spacing: 10) {
                                        if let d = cand.releaseDate, !d.isEmpty { Text(d) }
                                        if let m = cand.maker, !m.isEmpty { Text(m).lineLimit(1) }
                                        Text("\(cand.actors.count) 演员")
                                        Text("\(cand.sampleImages.count) 剧照")
                                    }
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)

                                    if !cand.actors.isEmpty {
                                        Text(cand.actors.joined(separator: " · "))
                                            .font(.caption)
                                            .foregroundStyle(.purple)
                                            .lineLimit(2)
                                    }

                                    Button {
                                        Task { await adopt(cand) }
                                    } label: {
                                        HStack(spacing: 6) {
                                            if applyingSource == cand.source {
                                                ProgressView().tint(.white)
                                            } else {
                                                Image(systemName: "checkmark.circle.fill")
                                            }
                                            Text(currentSource == cand.source ? "再次采用此源" : "采用此源资料")
                                        }
                                        .font(.subheadline.bold())
                                        .frame(maxWidth: .infinity)
                                    }
                                    .buttonStyle(.borderedProminent)
                                    .tint(.purple)
                                    .disabled(applyingSource != nil)
                                }
                                .padding(12)
                                .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            }
                        }
                        .padding(14)
                    }
                }
            }
            .navigationTitle("\(number) · 多源对比")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
            .task { await load() }
        }
    }

    private func load() async {
        guard let client = model.apiClient else { loading = false; return }
        loading = true
        errorMessage = nil
        do {
            candidates = try await client.queryMovieMultiSource(number: number)
        } catch {
            errorMessage = error.localizedDescription
        }
        loading = false
    }

    private func adopt(_ cand: MovieMetadata) async {
        guard let client = model.apiClient else { return }
        applyingSource = cand.source
        do {
            let saved = try await client.applyMovieMetadataSource(cand)
            onAdopted(saved)
        } catch {
            errorMessage = error.localizedDescription
        }
        applyingSource = nil
    }
}

/// 演员详情多源对比与选择 Sheet（维基百科 / gfriends / JavBus / JavDB）
struct PerformerMultiSourceSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let performerId: String
    let performerName: String
    let onAdopted: (MoviePerformer) -> Void

    @State private var candidates: [PerformerSourceCandidate] = []
    @State private var loading = true
    @State private var applyingKey: String?
    @State private var errorMessage: String?

    var body: some View {
        NavigationStack {
            Group {
                if loading {
                    ProgressView("正在并发查询 维基百科 / gfriends / JavBus / JavDB…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let errorMessage {
                    ContentUnavailableView("查询失败", systemImage: "exclamationmark.triangle", description: Text(errorMessage))
                } else if candidates.isEmpty {
                    ContentUnavailableView("未查到多源资料", systemImage: "person.slash", description: Text("暂未从各公开源匹配到该演员资料。"))
                } else {
                    ScrollView {
                        LazyVStack(spacing: 12) {
                            if candidates.count > 1 {
                                VStack(alignment: .leading, spacing: 8) {
                                    Text("已查到 \(candidates.count) 个互补数据源，可一键聚合高清头像、档案、别名与中文简介")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                    Button {
                                        Task { await adoptMerged() }
                                    } label: {
                                        Label(applyingKey == "merged:all" ? "正在聚合保存…" : "一键聚合保存（推荐）", systemImage: "sparkles")
                                            .font(.subheadline.bold())
                                            .frame(maxWidth: .infinity)
                                    }
                                    .buttonStyle(.borderedProminent)
                                    .tint(.purple)
                                    .disabled(applyingKey != nil)
                                }
                                .padding(12)
                                .background(Color.purple.opacity(0.12), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            }
                            ForEach(candidates) { cand in
                                VStack(alignment: .leading, spacing: 10) {
                                    HStack(alignment: .top, spacing: 12) {
                                        ZStack {
                                            Color.black.opacity(0.6)
                                            if let raw = cand.avatarUrl, let url = model.proxyScraperImageURL(rawUrl: raw) {
                                                CachedImageView(url: url, contentMode: .fill, minHeight: 64, maxPixel: 260)
                                            } else {
                                                Text(String(cand.name.prefix(1)))
                                                    .font(.title3.bold())
                                                    .foregroundStyle(.white)
                                            }
                                        }
                                        .frame(width: 64, height: 64)
                                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))

                                        VStack(alignment: .leading, spacing: 4) {
                                            Text(cand.sourceLabel)
                                                .font(.caption.bold())
                                                .padding(.horizontal, 8)
                                                .padding(.vertical, 3)
                                                .background(Color.purple.opacity(0.2), in: Capsule())
                                                .foregroundStyle(.purple)

                                            HStack(spacing: 8) {
                                                if let bd = cand.birthDate, !bd.isEmpty { Text("生日 \(bd)") }
                                                if let h = cand.heightCm, h > 0 { Text("\(h)cm") }
                                                if let m = cand.measurements, !m.isEmpty { Text(m) }
                                            }
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)

                                            if !cand.aliases.isEmpty {
                                                Text("别名：\(cand.aliases.joined(separator: " / "))")
                                                    .font(.caption2)
                                                    .foregroundStyle(.secondary)
                                                    .lineLimit(2)
                                            }
                                            if let bio = cand.bio, !bio.isEmpty {
                                                Text(bio)
                                                    .font(.caption2)
                                                    .foregroundStyle(.secondary)
                                                    .lineLimit(3)
                                            }
                                        }
                                    }

                                    HStack(spacing: 8) {
                                        Button {
                                            Task { await adopt(cand, avatarOnly: false) }
                                        } label: {
                                            Label("采用此源资料", systemImage: "checkmark")
                                                .font(.caption.bold())
                                                .frame(maxWidth: .infinity)
                                        }
                                        .buttonStyle(.borderedProminent)
                                        .tint(.purple)
                                        .disabled(applyingKey != nil)

                                        if cand.avatarUrl != nil {
                                            Button {
                                                Task { await adopt(cand, avatarOnly: true) }
                                            } label: {
                                                Label("仅用头像", systemImage: "photo")
                                                    .font(.caption.bold())
                                            }
                                            .buttonStyle(.bordered)
                                            .disabled(applyingKey != nil)
                                        }
                                    }
                                }
                                .padding(12)
                                .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            }
                        }
                        .padding(14)
                    }
                }
            }
            .navigationTitle("\(performerName) · 多源资料")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
            .task { await load() }
        }
    }

    private func load() async {
        guard let client = model.apiClient else { loading = false; return }
        loading = true
        errorMessage = nil
        do {
            candidates = try await client.queryPerformerMultiSource(id: performerId)
        } catch {
            errorMessage = error.localizedDescription
        }
        loading = false
    }

    private func adopt(_ cand: PerformerSourceCandidate, avatarOnly: Bool) async {
        guard let client = model.apiClient else { return }
        applyingKey = cand.source + (avatarOnly ? ":avatar" : ":all")
        do {
            let updated = try await client.applyPerformerSourceCandidate(id: performerId, candidate: cand, avatarOnly: avatarOnly)
            onAdopted(updated)
        } catch {
            errorMessage = error.localizedDescription
        }
        applyingKey = nil
    }

    private func adoptMerged() async {
        guard !candidates.isEmpty else { return }
        func pickString(_ order: [String], _ getter: (PerformerSourceCandidate) -> String?) -> String? {
            for src in order {
                if let hit = candidates.first(where: { $0.source == src }),
                   let val = getter(hit)?.trimmingCharacters(in: .whitespacesAndNewlines), !val.isEmpty {
                    return val
                }
            }
            for c in candidates {
                if let val = getter(c)?.trimmingCharacters(in: .whitespacesAndNewlines), !val.isEmpty {
                    return val
                }
            }
            return nil
        }
        func pickInt(_ order: [String], _ getter: (PerformerSourceCandidate) -> Int?) -> Int? {
            for src in order {
                if let hit = candidates.first(where: { $0.source == src }), let val = getter(hit), val > 0 {
                    return val
                }
            }
            for c in candidates {
                if let val = getter(c), val > 0 { return val }
            }
            return nil
        }
        var aliasSet = Set<String>()
        var mergedAliases: [String] = []
        for c in candidates {
            for a in c.aliases {
                let clean = a.trimmingCharacters(in: .whitespacesAndNewlines)
                if !clean.isEmpty, clean != performerName, aliasSet.insert(clean).inserted {
                    mergedAliases.append(clean)
                }
            }
        }
        let merged = PerformerSourceCandidate(
            source: "merged",
            sourceLabel: "多源聚合",
            name: performerName,
            aliases: mergedAliases,
            avatarUrl: pickString(["gfriends", "javbus", "javdb", "wikipedia"], { $0.avatarUrl }),
            profileUrl: pickString(["javbus", "javdb", "wikipedia"], { $0.profileUrl }),
            birthDate: pickString(["javbus", "javdb", "wikipedia"], { $0.birthDate }),
            heightCm: pickInt(["javbus", "javdb", "wikipedia"], { $0.heightCm }),
            measurements: pickString(["javbus", "javdb", "wikipedia"], { $0.measurements }),
            hometown: pickString(["javbus", "javdb", "wikipedia"], { $0.hometown }),
            hobby: pickString(["javbus", "javdb", "wikipedia"], { $0.hobby }),
            bloodType: pickString(["wikipedia", "javbus", "javdb"], { $0.bloodType }),
            debutYear: pickString(["wikipedia", "javbus", "javdb"], { $0.debutYear }),
            agency: pickString(["wikipedia", "javbus", "javdb"], { $0.agency }),
            bio: pickString(["wikipedia", "javbus", "javdb"], { $0.bio })
        )
        await adopt(merged, avatarOnly: false)
    }
}

/// 同系列作品查看 Sheet（Tab 1: 库内已有，Tab 2: 全网同系列 / 未入库）
struct MovieSeriesSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let series: String
    let onOpenLocalMovie: (MovieMetadata) -> Void
    let onPlayAsset: (String, String) -> Void

    private enum SeriesTab: String, CaseIterable, Identifiable {
        case local = "库内已有"
        case online = "全网 / 未入库"
        var id: String { rawValue }
    }

    @State private var tab: SeriesTab = .local
    @State private var localMovies: [MovieMetadata] = []
    @State private var localLoading = true
    @State private var onlineMovies: [PerformerRecentRelease] = []
    @State private var onlineLoading = false
    @State private var onlineLoaded = false
    @State private var onlyUnowned = true
    @State private var selectedOnline: PerformerRecentRelease?
    @State private var errorMessage: String?

    private var filteredOnline: [PerformerRecentRelease] {
        onlyUnowned ? onlineMovies.filter { !$0.inLibrary } : onlineMovies
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 10) {
                Picker("视图", selection: $tab) {
                    ForEach(SeriesTab.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .padding(.horizontal)
                .onChange(of: tab) { _, next in
                    if next == .online && !onlineLoaded && !onlineLoading {
                        Task { await loadOnline(force: false) }
                    }
                }

                if tab == .online {
                    HStack {
                        Toggle("仅看未入库", isOn: $onlyUnowned)
                            .font(.caption.weight(.semibold))
                        Spacer()
                        Button {
                            Task { await loadOnline(force: true) }
                        } label: {
                            Image(systemName: "arrow.clockwise")
                        }
                        .disabled(onlineLoading)
                    }
                    .padding(.horizontal)
                }

                if let errorMessage {
                    Text(errorMessage).font(.caption).foregroundStyle(.red).padding(.horizontal)
                }

                if tab == .local {
                    if localLoading {
                        ProgressView("正在加载库内同系列作品…").frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if localMovies.isEmpty {
                        ContentUnavailableView("暂无库内作品", systemImage: "film.stack")
                    } else {
                        ScrollView {
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                                ForEach(localMovies) { m in
                                    Button { onOpenLocalMovie(m) } label: {
                                        VStack(alignment: .leading, spacing: 5) {
                                            ZStack {
                                                Color.black.opacity(0.85)
                                                if let raw = m.coverUrl, let url = model.proxyScraperImageURL(rawUrl: raw) {
                                                    CachedImageView(url: url, contentMode: .fit, minHeight: 96, maxPixel: 480)
                                                }
                                            }
                                            .aspectRatio(3 / 2, contentMode: .fit)
                                            .clipShape(RoundedRectangle(cornerRadius: 8))
                                            Text(m.number).font(.caption2.bold()).foregroundStyle(.purple)
                                            Text(m.title).font(.caption).lineLimit(2).foregroundStyle(.primary)
                                        }
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                            .padding(.horizontal)
                        }
                    }
                } else {
                    if onlineLoading {
                        ProgressView("正在查询「\(series)」全网同系列…").frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else if filteredOnline.isEmpty {
                        ContentUnavailableView(
                            onlyUnowned && !onlineMovies.isEmpty ? "查到的作品均已入库" : "暂未查到同系列作品",
                            systemImage: "globe"
                        )
                    } else {
                        ScrollView {
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                                ForEach(filteredOnline) { item in
                                    Button { selectedOnline = item } label: {
                                        VStack(alignment: .leading, spacing: 5) {
                                            ZStack {
                                                Color.black.opacity(0.85)
                                                if let raw = item.coverUrl, let url = model.proxyScraperImageURL(rawUrl: raw) {
                                                    CachedImageView(url: url, contentMode: .fit, minHeight: 96, maxPixel: 480)
                                                }
                                            }
                                            .overlay(alignment: .topTrailing) {
                                                Text(item.inLibrary ? "库内已有" : "未入库")
                                                    .font(.system(size: 9, weight: .bold))
                                                    .padding(.horizontal, 5)
                                                    .padding(.vertical, 2)
                                                    .background((item.inLibrary ? Color.green : Color.orange).opacity(0.9), in: Capsule())
                                                    .foregroundStyle(.white)
                                                    .padding(5)
                                            }
                                            .aspectRatio(3 / 2, contentMode: .fit)
                                            .clipShape(RoundedRectangle(cornerRadius: 8))
                                            Text(item.number).font(.caption2.bold()).foregroundStyle(.purple)
                                            Text(item.title).font(.caption).lineLimit(2).foregroundStyle(.primary)
                                        }
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                            .padding(.horizontal)
                        }
                    }
                }
            }
            .navigationTitle(series)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) { Button("完成") { dismiss() } }
            }
            .sheet(item: $selectedOnline) { rel in
                OnlineMovieDetailSheet(release: rel, onPlayAsset: onPlayAsset)
                    .environmentObject(model)
            }
            .task { await loadLocal() }
        }
    }

    private func loadLocal() async {
        guard let client = model.apiClient else { localLoading = false; return }
        localLoading = true
        do { localMovies = try await client.listMoviesBySeries(series: series) }
        catch { errorMessage = error.localizedDescription }
        localLoading = false
    }

    private func loadOnline(force: Bool) async {
        guard let client = model.apiClient else { return }
        onlineLoading = true
        errorMessage = nil
        do {
            onlineMovies = try await client.getSeriesOnlineMovies(series: series, forceRefresh: force)
            onlineLoaded = true
        } catch {
            errorMessage = error.localizedDescription
        }
        onlineLoading = false
    }
}
