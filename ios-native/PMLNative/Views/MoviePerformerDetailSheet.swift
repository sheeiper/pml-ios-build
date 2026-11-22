import SwiftUI
import PhotosUI
import UIKit

struct MoviePerformerDetailSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let performer: MoviePerformer
    var onFilterMovies: ((String) -> Void)? = nil
    var onPlay: ((String, String) -> Void)? = nil

    @State private var detail: MoviePerformerDetail?
    @State private var loading = false
    @State private var saving = false
    @State private var enriching = false
    @State private var errorMessage: String?
    @State private var showEditor = false
    @State private var selectedMovie: MovieMetadata?

    var body: some View {
        NavigationStack {
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
                        profileHeader(person)

                        HStack(spacing: 8) {
                            Button {
                                Task { await enrich(force: !(person.avatarUrl ?? "").isEmpty) }
                            } label: {
                                Label(
                                    enriching ? "获取中…" : ((person.avatarUrl ?? "").isEmpty ? "自动获取资料" : "刷新网络资料"),
                                    systemImage: "sparkles"
                                )
                                .font(.caption.weight(.semibold))
                            }
                            .buttonStyle(.borderedProminent)
                            .tint(.purple)
                            .disabled(enriching)

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

                        if let bio = person.bio, !bio.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            Text(bio)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                                .padding(12)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(.quaternary.opacity(0.2), in: RoundedRectangle(cornerRadius: 10))
                        }

                        HStack {
                            Text("参演作品 (\(detail?.total ?? person.movieCount))")
                                .font(.headline)
                            Spacer()
                            if let movies = detail?.movies, (detail?.total ?? person.movieCount) > movies.count {
                                Text("前 \(movies.count) 部")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                        }

                        if let movies = detail?.movies, !movies.isEmpty {
                            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 12) {
                                ForEach(movies) { movie in
                                    movieCard(movie)
                                }
                            }
                        } else if !loading {
                            Text("暂无关联作品").font(.footnote).foregroundStyle(.secondary)
                        }
                    }
                }
                .padding(16)
            }
            .navigationTitle(detail?.person.name ?? performer.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("完成") { dismiss() } } }
            .task { await load() }
            .sheet(isPresented: $showEditor) {
                MoviePerformerEditor(person: detail?.person ?? performer) { await save($0) }
                    .environmentObject(model)
            }
            .sheet(item: $selectedMovie) { movie in
                MovieDetailSheet(number: movie.number, assetId: movie.assetId, fileName: movie.title, remotePath: nil) { targetId in
                    onPlay?(targetId, movie.title)
                }
                .environmentObject(model)
            }
        }
    }

    private func profileHeader(_ person: MoviePerformer) -> some View {
        HStack(spacing: 14) {
            PerformerAvatarView(value: person.avatarUrl, proxy: avatarURL(person.avatarUrl), fallbackName: person.name)
                .frame(width: 82, height: 82)
                .clipShape(Circle())
                .overlay(Circle().stroke(Color.purple.opacity(0.4), lineWidth: 2))
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
            }
            Spacer()
        }
        .padding(14)
        .background(.quaternary.opacity(0.22), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }

    private func movieCard(_ movie: MovieMetadata) -> some View {
        Button { selectedMovie = movie } label: {
            VStack(alignment: .leading, spacing: 5) {
                ZStack(alignment: .topLeading) {
                    Color.black.opacity(0.85)
                    CachedImageView(url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fit, minHeight: 96, maxPixel: 600)
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

                Text(movie.title)
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
}

private struct PerformerAvatarView: View {
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
