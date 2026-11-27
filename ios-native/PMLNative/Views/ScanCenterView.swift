import SwiftUI

struct ScanCenterView: View {
    @EnvironmentObject private var model: AppModel

    @State private var modules: [LibraryModuleInfo] = []
    @State private var syncingModuleId: String? = nil
    @State private var rebuilding = false
    @State private var progress: ScanProgress?
    @State private var cache: CacheStats?
    @State private var busy = false
    @State private var message = ""
    @State private var errorText = ""
    @State private var pollTask: Task<Void, Never>?

    var body: some View {
        List {
            Section {
                if modules.isEmpty {
                    Text("加载模块列表…").foregroundStyle(.secondary)
                } else {
                    ForEach(modules) { mod in
                        HStack(spacing: 12) {
                            Image(systemName: mod.symbol)
                                .font(.title3)
                                .foregroundStyle(.tint)
                                .frame(width: 28)

                            VStack(alignment: .leading, spacing: 3) {
                                HStack(spacing: 6) {
                                    Text(mod.name)
                                        .font(.subheadline.weight(.semibold))
                                    if !mod.enabled {
                                        Text("已停用")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                Text("\(mod.entityCount) \(mod.entityUnitLabel) · \(mod.assetCount) 文件 · \(PMLFormatters.bytes(mod.totalBytes))")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Text(mod.remotePath)
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }

                            Spacer(minLength: 6)

                            Button {
                                Task { await scanSingleModule(mod) }
                            } label: {
                                if syncingModuleId == mod.id {
                                    ProgressView()
                                } else {
                                    Text("同步")
                                        .font(.caption.weight(.semibold))
                                }
                            }
                            .buttonStyle(.bordered)
                            .disabled(syncingModuleId != nil || rebuilding || busy)
                        }
                        .padding(.vertical, 3)
                    }

                    Button {
                        Task { await rebuildAllIndexes() }
                    } label: {
                        HStack {
                            Label(rebuilding ? "重建索引中…" : "仅重建全部模块索引", systemImage: "arrow.triangle.2.circlepath")
                            Spacer()
                            if rebuilding { ProgressView() }
                        }
                    }
                    .disabled(syncingModuleId != nil || rebuilding || busy)
                }
            } header: {
                Text("五大模块独立同步")
            } footer: {
                Text("可单独同步某个模块目录，避免全库扫描耗时。")
            }

            Section("全库扫描状态") {
                if let progress {
                    LabeledContent("状态", value: progress.running ? "扫描中" : "空闲")
                    ProgressView(value: progress.percent, total: 100)
                    Text(progress.message)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if let error = progress.error, !error.isEmpty {
                        Text(error).font(.caption).foregroundStyle(.red)
                    }
                } else {
                    Text("尚未获取到进度").foregroundStyle(.secondary)
                }
            }

            Section("全库操作") {
                Button(busy ? "处理中…" : "全库扫描资料库") {
                    Task { await startScan() }
                }
                .disabled(busy || syncingModuleId != nil)
                Button("重扫失败的目录") {
                    Task { await rescanFailed() }
                }
                .disabled(busy || syncingModuleId != nil)
            }

            Section("索引统计") {
                if let cache {
                    LabeledContent("已索引文件", value: "\(cache.totalIndexedFiles)")
                    LabeledContent("已索引目录", value: "\(cache.totalIndexedDirectories)")
                    LabeledContent("数据库", value: pmlFormatBytes(cache.databaseBytes))
                    if let last = cache.lastScanTime {
                        LabeledContent(
                            "上次扫描",
                            value: Date(timeIntervalSince1970: TimeInterval(last))
                                .formatted(date: .abbreviated, time: .shortened)
                        )
                    }
                } else {
                    Text("加载中…").foregroundStyle(.secondary)
                }
            }

            if !message.isEmpty {
                Section {
                    Text(message).font(.footnote).foregroundStyle(.green)
                }
            }
            if !errorText.isEmpty {
                Section {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                }
            }
        }
        .navigationTitle("扫描中心")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
        .onDisappear { pollTask?.cancel() }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        async let modReq = client.listLibraryModules()
        async let progReq = client.scanProgress()
        async let cacheReq = client.cacheStats()
        modules = (try? await modReq) ?? []
        progress = try? await progReq
        cache = try? await cacheReq
        if progress?.running == true {
            startPolling()
        }
    }

    private func scanSingleModule(_ mod: LibraryModuleInfo) async {
        guard let client = model.apiClient else { return }
        syncingModuleId = mod.id
        message = ""
        errorText = ""
        defer { syncingModuleId = nil }
        do {
            let res = try await client.scanLibraryModule(moduleId: mod.id, full: false)
            message = "「\(mod.name)」增量同步完成：新增 +\(res.scan.added) · 更新 \(res.scan.updated) · 删除 -\(res.scan.deleted)（共扫描 \(res.scan.scanned) 项，索引 \(res.rebuild.entityCount) \(mod.entityUnitLabel)）"
            await load()
        } catch {
            errorText = "同步「\(mod.name)」失败：\(error.localizedDescription)"
        }
    }

    private func rebuildAllIndexes() async {
        guard let client = model.apiClient else { return }
        rebuilding = true
        message = ""
        errorText = ""
        defer { rebuilding = false }
        do {
            let summaries = try await client.rebuildModuleIndexes()
            let totalTagged = summaries.reduce(0) { $0 + $1.taggedAssets }
            message = "已重建 \(summaries.count) 个模块索引（共标记 \(totalTagged) 个资产）"
            await load()
        } catch {
            errorText = "重建索引失败：\(error.localizedDescription)"
        }
    }

    private func startScan() async {
        guard let client = model.apiClient else { return }
        busy = true
        errorText = ""
        do {
            let targets = try await scanTargets(client)
            guard !targets.isEmpty else {
                errorText = "请先在「媒体源」配置已整理与漫画目录"
                busy = false
                return
            }
            for root in targets {
                try await client.startScan(rootId: root.id)
                message = "扫描 \(root.remotePath)…"
                for _ in 0..<1800 {
                    try? await Task.sleep(nanoseconds: 1_000_000_000)
                    if let latest = try? await client.scanProgress() {
                        progress = latest
                        if !latest.running {
                            if let error = latest.error, !error.isEmpty {
                                errorText = error
                            }
                            break
                        }
                    }
                }
            }
            _ = try? await client.rebuildModuleIndexes()
            await load()
            message = "全库扫描与模块索引重建完成"
        } catch {
            errorText = error.localizedDescription
        }
        busy = false
    }

    private func scanTargets(_ client: PMLAPIClient) async throws -> [LibraryRoot] {
        let config = try await client.directoryConfig()
        let roots = try await client.libraryRoots()
        var targets: [LibraryRoot] = []
        if let vault = config.vaultPath,
           let root = roots.first(where: { $0.remotePath == vault && $0.kind == "media" }) {
            targets.append(root)
        }
        if let comics = config.comicsPath,
           let root = roots.first(where: { $0.remotePath == comics && $0.kind == "comics" }) {
            targets.append(root)
        }
        return targets
    }

    private func rescanFailed() async {
        guard let client = model.apiClient else { return }
        busy = true
        errorText = ""
        do {
            let result = try await client.rescanFailedDirectories()
            message = result.message
            cache = try? await client.cacheStats()
        } catch {
            errorText = error.localizedDescription
        }
        busy = false
    }

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task { @MainActor in
            guard let client = model.apiClient else { return }
            for _ in 0..<1800 {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                guard !Task.isCancelled else { return }
                if let latest = try? await client.scanProgress() {
                    progress = latest
                    if !latest.running {
                        cache = try? await client.cacheStats()
                        message = latest.error ?? "扫描完成"
                        return
                    }
                }
            }
        }
    }
}
