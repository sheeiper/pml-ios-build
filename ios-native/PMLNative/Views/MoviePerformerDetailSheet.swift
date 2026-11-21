import SwiftUI
import PhotosUI

/// Candidate detail intentionally keeps identity confirmation separate from profile editing.
struct MoviePerformerDetailSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let performer: MoviePerformer
    @State private var detail: MoviePerformerDetail?
    @State private var loading = false
    @State private var saving = false
    @State private var errorMessage: String?
    @State private var showEditor = false
    @State private var showConfirmAlert = false
    @State private var avatarItem: PhotosPickerItem?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if loading && detail == nil { ProgressView("加载人物候选…").frame(maxWidth: .infinity).padding(.vertical, 40) }
                    if let errorMessage { Label(errorMessage, systemImage: "exclamationmark.triangle").font(.footnote).foregroundStyle(.red) }
                    if let person = detail?.person ?? Optional(performer) {
                        profileHeader(person)
                        HStack {
                            Label(person.status == "confirmed" ? "已人工确认" : "未确认身份", systemImage: person.status == "confirmed" ? "checkmark.seal.fill" : "questionmark.diamond")
                                .foregroundStyle(person.status == "confirmed" ? .green : .orange)
                            Spacer()
                            Button("编辑资料") { showEditor = true }.buttonStyle(.bordered)
                        }
                        if person.status != "confirmed" {
                            Button { showConfirmAlert = true } label: { Label("我已核对并确认此身份", systemImage: "checkmark.seal") }
                                .buttonStyle(.borderedProminent).tint(.purple)
                        } else {
                            Button("撤销人工确认", role: .destructive) { Task { await setStatus("unconfirmed") } }.buttonStyle(.bordered)
                        }
                        Text("关联作品 (\(detail?.total ?? person.movieCount))") .font(.headline)
                        if let movies = detail?.movies, !movies.isEmpty {
                            LazyVStack(alignment: .leading, spacing: 10) { ForEach(movies) { movie in movieRow(movie) } }
                        } else { Text("暂无已验证关联作品").foregroundStyle(.secondary) }
                    }
                }.padding(16)
            }
            .navigationTitle("人物候选详情")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("完成") { dismiss() } } }
            .task { await load() }
            .sheet(isPresented: $showEditor) {
                MoviePerformerEditor(person: detail?.person ?? performer) { await save($0) }
                    .environmentObject(model)
            }
            .alert("确认身份？", isPresented: $showConfirmAlert) {
                Button("确认") { Task { await setStatus("confirmed") } }
                Button("取消", role: .cancel) {}
            } message: {
                Text("这只会确认当前候选，不会猜测、合并其他人物，也不会批量修改作品。")
            }
        }
    }

    private func profileHeader(_ person: MoviePerformer) -> some View {
        HStack(spacing: 14) {
            CachedImageView(url: avatarURL(person.avatarUrl), contentMode: .fill, minHeight: 76, maxPixel: 360)
                .frame(width: 76, height: 76).clipShape(Circle())
            VStack(alignment: .leading, spacing: 5) {
                Text(person.name).font(.title2.bold())
                if !person.aliases.isEmpty { Text(person.aliases.joined(separator: " / ")).font(.footnote).foregroundStyle(.secondary) }
                Text("来源：\(person.source) · 仅本地候选").font(.caption2).foregroundStyle(.secondary)
            }
        }
    }
    private func movieRow(_ movie: MovieMetadata) -> some View {
        HStack(spacing: 10) {
            CachedImageView(url: movie.coverUrl.flatMap { model.proxyScraperImageURL(rawUrl: $0) }, contentMode: .fill, minHeight: 54, maxPixel: 300)
                .frame(width: 96, height: 54).clipShape(RoundedRectangle(cornerRadius: 6))
            VStack(alignment: .leading) { Text(movie.number).font(.caption.bold()); Text(movie.title).font(.caption).lineLimit(2) }
            Spacer()
        }
    }
    private func avatarURL(_ value: String?) -> URL? {
        guard let value, !value.isEmpty else { return nil }
        if value.hasPrefix("data:") { return URL(string: value) }
        return model.proxyScraperImageURL(rawUrl: value)
    }
    private func load() async {
        guard let client = model.apiClient else { errorMessage = "未连接到数据源"; return }
        loading = true; defer { loading = false }
        do { detail = try await client.getMoviePerformer(id: performer.id) } catch { errorMessage = error.localizedDescription }
    }
    private func setStatus(_ status: String) async {
        guard let client = model.apiClient else { return }; saving = true; defer { saving = false }
        do { let person = try await client.setMoviePerformerStatus(id: performer.id, status: status); detail = MoviePerformerDetail(person: person, movies: detail?.movies ?? [], total: detail?.total ?? person.movieCount) } catch { errorMessage = error.localizedDescription }
    }
    private func save(_ input: MoviePerformerEditor.Input) async {
        guard let client = model.apiClient else { return }; saving = true; defer { saving = false }
        do { let person = try await client.updateMoviePerformer(id: performer.id, name: input.name, aliases: input.aliases, avatarURL: input.avatarURL, profileURL: input.profileURL); detail = MoviePerformerDetail(person: person, movies: detail?.movies ?? [], total: detail?.total ?? person.movieCount); showEditor = false } catch { errorMessage = error.localizedDescription }
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
                    Text("服务端限制 JPEG/PNG/WebP，最大 2 MiB；不会自动识别人脸。").font(.caption2).foregroundStyle(.secondary)
                    if !notice.isEmpty { Text(notice).font(.caption).foregroundStyle(.secondary) }
                }
                Button("保存") { Task { await save() } }.disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }.navigationTitle("编辑人物资料").navigationBarTitleDisplayMode(.inline).toolbar { ToolbarItem(placement: .topBarLeading) { Button("取消") { dismiss() } } }
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
