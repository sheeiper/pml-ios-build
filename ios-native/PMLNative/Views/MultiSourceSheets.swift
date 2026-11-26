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
                    ProgressView("正在并发查询 JavDB / JavBus / Jav321 / AV-Wiki…")
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
}
