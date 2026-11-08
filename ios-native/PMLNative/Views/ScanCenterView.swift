import SwiftUI

struct ScanCenterView: View {
    @EnvironmentObject private var model: AppModel

    @State private var progress: ScanProgress?
    @State private var cache: CacheStats?
    @State private var busy = false
    @State private var message = ""
    @State private var errorText = ""
    @State private var pollTask: Task<Void, Never>?

    var body: some View {
        List {
            Section("扫描状态") {
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

            Section("操作") {
                Button(busy ? "处理中…" : "立即扫描资料库") {
                    Task { await startScan() }
                }
                .disabled(busy)
                Button("重扫失败的目录") {
                    Task { await rescanFailed() }
                }
                .disabled(busy)
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
                    Text(message).font(.footnote).foregroundStyle(.secondary)
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
        .task { await load() }
        .onDisappear { pollTask?.cancel() }
    }

    private func load() async {
        guard let client = model.apiClient else { return }
        progress = try? await client.scanProgress()
        cache = try? await client.cacheStats()
        if progress?.running == true {
            startPolling()
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
            cache = try? await client.cacheStats()
            message = "扫描完成"
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
