import SwiftUI

/// 多模块资料库配置页（对应 Web 端 LibraryModulesSettingsCard）。
struct LibraryModulesSettingsView: View {
    @EnvironmentObject private var model: AppModel

    @State private var modules: [LibraryModuleInfo] = []
    @State private var loading = false
    @State private var syncingId: String? = nil
    @State private var editingModule: LibraryModuleInfo? = nil
    @State private var showAddSheet = false
    @State private var noticeText = ""
    @State private var errorText = ""

    var body: some View {
        List {
            if !noticeText.isEmpty {
                Section {
                    Text(noticeText).font(.footnote).foregroundStyle(.green)
                }
            }
            if !errorText.isEmpty {
                Section {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                }
            }

            Section {
                if loading && modules.isEmpty {
                    ProgressView("加载模块…")
                } else {
                    ForEach(modules) { mod in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Label(mod.name, systemImage: mod.symbol)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(mod.kindLabel)
                                    .font(.caption2)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 3)
                                    .background(Color.accentColor.opacity(0.14), in: Capsule())
                            }

                            Text(mod.remotePath)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)

                            HStack {
                                Text("\(mod.entityCount) \(mod.entityUnitLabel) · \(mod.assetCount) 文件 · \(PMLFormatters.bytes(mod.totalBytes))")
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                Spacer()
                                Button("编辑") {
                                    editingModule = mod
                                }
                                .font(.caption)
                                .buttonStyle(.bordered)

                                Button {
                                    Task { await syncModule(mod) }
                                } label: {
                                    if syncingId == mod.id {
                                        ProgressView()
                                    } else {
                                        Text("同步")
                                    }
                                }
                                .font(.caption)
                                .buttonStyle(.borderedProminent)
                                .disabled(syncingId != nil)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                }
            } header: {
                Text("已配置模块（\(modules.count)）")
            } footer: {
                Text("每个模块独立映射 115 目录并支持单独增量同步。")
            }
        }
        .navigationTitle("多模块资料库")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showAddSheet = true
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel("新增模块")
            }
        }
        .refreshable { await load() }
        .task { await load() }
        .sheet(item: $editingModule) { mod in
            EditLibraryModuleSheet(existing: mod) {
                Task { await load() }
            }
        }
        .sheet(isPresented: $showAddSheet) {
            EditLibraryModuleSheet(existing: nil) {
                Task { await load() }
            }
        }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            modules = try await client.listLibraryModules()
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func syncModule(_ mod: LibraryModuleInfo) async {
        guard let client = model.apiClient else { return }
        syncingId = mod.id
        noticeText = ""
        errorText = ""
        defer { syncingId = nil }
        do {
            let res = try await client.scanLibraryModule(moduleId: mod.id, full: false)
            noticeText = "「\(mod.name)」同步完成：扫描 \(res.scan.scanned) 项，新增 \(res.scan.added)"
            await load()
        } catch {
            errorText = error.localizedDescription
        }
    }
}

private struct EditLibraryModuleSheet: View {
    let existing: LibraryModuleInfo?
    let onSaved: () -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var id: String
    @State private var kind: String
    @State private var name: String
    @State private var remotePath: String
    @State private var enabled: Bool
    @State private var saving = false
    @State private var errorText = ""

    private let kinds = [
        ("movies", "影视库"),
        ("creators", "创作者"),
        ("collections", "专题合集"),
        ("galleries", "写真图集"),
        ("comics", "漫画库"),
    ]

    init(existing: LibraryModuleInfo?, onSaved: @escaping () -> Void) {
        self.existing = existing
        self.onSaved = onSaved
        _id = State(initialValue: existing?.id ?? "")
        _kind = State(initialValue: existing?.kind ?? "collections")
        _name = State(initialValue: existing?.name ?? "")
        _remotePath = State(initialValue: existing?.remotePath ?? "/115open/文件/")
        _enabled = State(initialValue: existing?.enabled ?? true)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section("模块属性") {
                    TextField("模块 ID（英文，如 galleries）", text: $id)
                        .disabled(existing != nil)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Picker("模块类型", selection: $kind) {
                        ForEach(kinds, id: \.0) { item in
                            Text(item.1).tag(item.0)
                        }
                    }
                    TextField("显示名称", text: $name)
                    TextField("115 远程路径", text: $remotePath)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Toggle("启用该模块", isOn: $enabled)
                }

                if !errorText.isEmpty {
                    Section {
                        Text(errorText).font(.footnote).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle(existing == nil ? "新增模块" : "编辑模块")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("取消") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(saving ? "保存中…" : "保存") {
                        Task { await save() }
                    }
                    .disabled(saving)
                }
            }
        }
    }

    private func save() async {
        guard let client = model.apiClient else { return }
        saving = true
        errorText = ""
        defer { saving = false }
        do {
            _ = try await client.upsertLibraryModule(
                id: id.trimmingCharacters(in: .whitespacesAndNewlines),
                kind: kind,
                name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                remotePath: remotePath.trimmingCharacters(in: .whitespacesAndNewlines),
                domain: kind == "comics" ? "comics" : "vault",
                enabled: enabled,
                sortOrder: existing?.sortOrder
            )
            onSaved()
            dismiss()
        } catch {
            errorText = error.localizedDescription
        }
    }
}
