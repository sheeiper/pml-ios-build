import SwiftUI

/// 待收纳整理：发现【PendingX】批次，逐条确认目标目录后移动到资料库。
struct PendingCurationView: View {
    @EnvironmentObject private var model: AppModel

    @State private var pendingPath = ""
    @State private var vaultRoot = ""

    @State private var roots: [LibraryRoot] = []
    @State private var batches: [RemoteBrowserEntry] = []
    @State private var selectedBatch: RemoteBrowserEntry?
    @State private var items: [PendingItem] = []
    @State private var currentIndex = 0

    @State private var authors: [LibraryAuthor] = []
    @State private var categories: [LibraryCategory] = []

    @State private var loading = false
    @State private var busy = false
    @State private var message = ""
    @State private var errorText = ""

    @State private var previewURL: URL?
    @State private var editingIndex: Int?
    @State private var pickerCategoryId = ""
    @State private var pickerAuthorId = ""
    @State private var pickerPath = ""
    @State private var inboxPath = ""
    @State private var inboxBatchName = ""
    @State private var packing = false
    @State private var processedCount = 0
    @State private var moveSummary: [String] = []

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                configCard
                batchCard

                if !errorText.isEmpty {
                    Text(errorText).font(.footnote).foregroundStyle(.red)
                }
                if !message.isEmpty {
                    Text(message).font(.footnote).foregroundStyle(.secondary)
                }

                if selectedBatch != nil {
                    deckCard
                    actionsCard
                }
            }
            .padding(16)
        }
        .navigationTitle("待整理")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { await reloadAll() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .disabled(loading)
            }
        }
        .task { await reloadAll() }
        .sheet(isPresented: Binding(
            get: { editingIndex != nil },
            set: { if !$0 { editingIndex = nil } }
        )) {
            targetPicker
        }
    }

    // MARK: - 配置

    private var configCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("待整理目录").font(.headline)
            Text(pendingPath.isEmpty ? "未配置，请先到媒体源选择待整理目录" : pendingPath)
                .font(.footnote)
                .foregroundStyle(pendingPath.isEmpty ? .orange : .secondary)
                .lineLimit(2)
            Text("收纳目标：\(vaultRoot.isEmpty ? "未配置" : vaultRoot)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            NavigationLink {
                MediaSourcesView()
            } label: {
                Label("去设置媒体源", systemImage: "gearshape")
                    .font(.caption)
            }
        }
        .padding(14)
        .pmlGlassCard(cornerRadius: 18)
    }

    private var batchCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("待收纳批次").font(.headline)
                Text("\(batches.count)").font(.caption).foregroundStyle(.secondary)
                Spacer()
            }
            if batches.isEmpty {
                Text("当前待收纳目录下没有【PendingX】批次目录")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(batches) { batch in
                            Button {
                                Task { await select(batch) }
                            } label: {
                                Text(batch.name)
                                    .font(.caption)
                                    .padding(.horizontal, 12)
                                    .padding(.vertical, 7)
                                    .background(
                                        selectedBatch?.id == batch.id ? Color.accentColor : Color(.secondarySystemBackground),
                                        in: Capsule()
                                    )
                                    .foregroundStyle(selectedBatch?.id == batch.id ? Color.white : Color.primary)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
            }
        }
        .padding(14)
        .pmlGlassCard(cornerRadius: 18)
    }

    // MARK: - 逐条确认

    private var currentItem: PendingItem? {
        items.indices.contains(currentIndex) ? items[currentIndex] : nil
    }

    private var deckCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(currentItem?.entry.name ?? "已全部处理")
                    .font(.headline)
                    .lineLimit(2)
                Spacer()
                if !items.isEmpty {
                    Text("\(currentIndex + 1)/\(items.count)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if let item = currentItem {
                ZStack {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(.quaternary.opacity(0.3))
                    if let previewURL, !item.entry.isDirectory {
                        CachedImageView(url: previewURL, contentMode: .fill, minHeight: 180)
                    } else {
                        Image(systemName: item.entry.isDirectory ? "folder.fill" : "doc.fill")
                            .font(.system(size: 42))
                            .foregroundStyle(.tint)
                    }
                }
                .frame(height: 180)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                LabeledContent("类型", value: item.entry.isDirectory ? "文件夹" : "文件")
                LabeledContent("建议目标", value: item.targetDir ?? "未匹配，请手动选择")
            }

            if let item = currentItem {
                Text("\(item.entry.isDirectory ? "文件夹" : "文件") · \(pmlFormatBytes(UInt64(max(0, item.entry.sizeBytes))))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let author = item.suggestedAuthor {
                    Text("建议作者：\(author)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            HStack(spacing: 10) {
                Button("收纳到此") { Task { await moveCurrent() } }
                    .buttonStyle(.borderedProminent)
                    .disabled(busy || currentItem == nil || currentItem?.targetDir == nil)
                Button("改目标") { openPicker() }
                    .buttonStyle(.bordered)
                    .disabled(currentItem == nil)
                Button("跳过") { skipCurrent() }
                    .buttonStyle(.bordered)
                    .disabled(items.isEmpty)
                Spacer()
            }
        }
        .padding(14)
        .pmlGlassCard(cornerRadius: 20)
        .task(id: currentItem?.id) {
            await loadPreview()
        }
    }

    private var actionsCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("批量操作").font(.headline)
            let ready = items.filter { $0.targetDir != nil }.count
            ProgressView(value: Double(processedCount), total: Double(max(1, processedCount + items.count)))
            Text("已处理 \(processedCount) 项 · 待处理 \(items.count) 项 · 可收纳 \(ready) 项")
                .font(.caption)
                .foregroundStyle(.secondary)
            Button("按目标收纳全部（\(ready) 项）") { Task { await moveAll() } }
                .buttonStyle(.borderedProminent)
                .disabled(busy || ready == 0)

            Button("清理该目录已失效的媒体索引") { Task { await purge() } }
                .buttonStyle(.bordered)
            Button("删除空的批次目录") { Task { await deleteEmptyBatch() } }
                .buttonStyle(.bordered)
                .disabled(selectedBatch == nil)

            Divider().padding(.vertical, 4)

            Text("把 Inbox 打包成新批次").font(.subheadline.weight(.semibold))
            TextField("Inbox 目录，例如 /115open/待收纳/inbox", text: $inboxPath)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("批次名称（留空自动生成）", text: $inboxBatchName)
                .textFieldStyle(.roundedBorder)
            Button(packing ? "打包中…" : "列出 Inbox 文件并打包") { Task { await packInbox() } }
                .buttonStyle(.bordered)
                .disabled(packing || inboxPath.isEmpty)
        }
        .padding(14)
        .pmlGlassCard(cornerRadius: 20)
    }

    // MARK: - 目标选择

    private var targetPicker: some View {
        NavigationStack {
            Form {
                Section("目标目录") {
                    Picker("分类", selection: $pickerCategoryId) {
                        Text("不指定").tag("")
                        ForEach(categories) { category in
                            Text(category.name).tag(category.id)
                        }
                    }
                    Picker("作者", selection: $pickerAuthorId) {
                        Text("不指定").tag("")
                        ForEach(authors) { author in
                            Text(author.name).tag(author.id)
                        }
                    }
                    TextField("完整目标目录", text: $pickerPath)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section {
                    Button("保存目标") { applyPicker() }
                }
            }
            .navigationTitle("选择收纳目标")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { editingIndex = nil }
                }
            }
        }
    }

    private func openPicker() {
        guard let item = currentItem else { return }
        editingIndex = currentIndex
        pickerPath = item.targetDir ?? vaultRoot
        pickerCategoryId = ""
        pickerAuthorId = ""
    }

    private func applyPicker() {
        guard let index = editingIndex, items.indices.contains(index) else {
            editingIndex = nil
            return
        }
        let author = authors.first { $0.id == pickerAuthorId }
        let category = categories.first { $0.id == pickerCategoryId }
        var path = pickerPath.trimmingCharacters(in: .whitespacesAndNewlines)
        if path.isEmpty, !vaultRoot.isEmpty {
            path = vaultRoot
            if let category { path += "/\(category.name)" }
            if let author { path += "/【\(author.name)】" }
        }
        items[index].targetDir = path.isEmpty ? nil : path
        if let author { items[index].suggestedAuthor = author.name }
        editingIndex = nil
    }

    // MARK: - 数据

    private func reloadAll() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            let config = try await client.directoryConfig()
            pendingPath = config.pendingPath ?? ""
            vaultRoot = config.vaultPath ?? ""
            roots = try await client.libraryRoots()
            authors = try await client.libraryAuthors(filter: "all")
            categories = try await client.libraryCategories(filter: "all")
            await discoverBatches()
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }

    private func discoverBatches() async {
        guard let client = model.apiClient, !pendingPath.isEmpty else { return }
        guard let root = root(for: pendingPath) else {
            batches = []
            return
        }
        do {
            let page = try await client.remoteChildren(rootId: root.id, path: pendingPath, refresh: true)
            batches = page.items
                .filter { $0.isDirectory && pmlIsPendingFolder($0.name) }
                .sorted { $0.name.localizedCompare($1.name) == .orderedAscending }
            if let first = batches.first {
                await select(first)
            } else {
                selectedBatch = nil
                items = []
            }
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
    }

    private func select(_ batch: RemoteBrowserEntry) async {
        guard let client = model.apiClient, let root = root(for: batch.path) else { return }
        selectedBatch = batch
        message = ""
        do {
            let page = try await client.remoteChildren(rootId: root.id, path: batch.path, refresh: true)
            items = page.items.map { entry in
                var item = PendingItem(entry: entry, targetDir: nil, suggestedAuthor: nil)
                let suggestion = suggest(for: entry.name)
                item.targetDir = suggestion.dir
                item.suggestedAuthor = suggestion.author
                return item
            }
            currentIndex = 0
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func loadPreview() async {
        guard let client = model.apiClient, let item = currentItem, !item.entry.isDirectory else {
            previewURL = nil
            return
        }
        previewURL = try? await client.rawMediaURL(remotePath: item.entry.path)
    }

    private func saveConfig() async {
        guard let client = model.apiClient else { return }
        do {
            try await client.saveCuratorConfig(
                pendingPath: pendingPath.isEmpty ? nil : pendingPath,
                targetVaultRoot: vaultRoot.isEmpty ? nil : vaultRoot
            )
            message = "收纳配置已保存"
            await discoverBatches()
        } catch {
            errorText = error.localizedDescription
        }
    }

    // MARK: - 移动

    private func moveCurrent() async {
        guard let client = model.apiClient,
              let item = currentItem,
              let dir = item.targetDir else { return }
        busy = true
        defer { busy = false }
        do {
            let result = try await client.curateMoveItem(
                sourcePath: item.entry.path,
                targetPath: "\(dir)/\(item.entry.name)",
                isDirectory: item.entry.isDirectory
            )
            message = result.message
            moveSummary.insert("\(item.entry.name)：\(result.message)", at: 0)
            if result.success || result.status == "SKIPPED_EXIST" {
                processedCount += 1
                items.remove(at: currentIndex)
                if currentIndex >= items.count { currentIndex = max(0, items.count - 1) }
            }
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func moveAll() async {
        guard let client = model.apiClient else { return }
        let ready = items.filter { $0.targetDir != nil }
        guard !ready.isEmpty else { return }
        busy = true
        defer { busy = false }
        do {
            let payload = ready.map { item in
                (source: item.entry.path, target: "\(item.targetDir!)/\(item.entry.name)", isDirectory: item.entry.isDirectory)
            }
            let result = try await client.curateBatchMove(payload)
            message = "成功 \(result.successCount)，跳过 \(result.skippedCount)，失败 \(result.failedCount)"
            processedCount += result.successCount + result.skippedCount
            for item in result.results {
                moveSummary.insert("\(item.sourcePath.split(separator: "/").last.map(String.init) ?? item.sourcePath)：\(item.message)", at: 0)
            }
            let movedSources = Set(result.results.filter { $0.success || $0.status == "SKIPPED_EXIST" }.map(\.sourcePath))
            items.removeAll { movedSources.contains($0.entry.path) }
            if currentIndex >= items.count { currentIndex = max(0, items.count - 1) }
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func purge() async {
        guard let client = model.apiClient else { return }
        busy = true
        defer { busy = false }
        do {
            try await client.purgePendingAssets(pendingPath: pendingPath)
            message = "已清理待收纳目录的失效索引"
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func deleteEmptyBatch() async {
        guard let client = model.apiClient, let batch = selectedBatch else { return }
        busy = true
        defer { busy = false }
        do {
            try await client.deleteEmptyPendingBatch(batchPath: batch.path)
            message = "已删除空批次 \(batch.name)"
            await discoverBatches()
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func skipCurrent() {
        guard !items.isEmpty else { return }
        currentIndex = (currentIndex + 1) % items.count
    }

    private func packInbox() async {
        guard let client = model.apiClient else { return }
        guard let root = root(for: inboxPath) else {
            errorText = "找不到 Inbox 所属的资料库根目录"
            return
        }
        packing = true
        errorText = ""
        do {
            let page = try await client.remoteChildren(rootId: root.id, path: inboxPath, refresh: true)
            let names = page.items.filter { !$0.isDirectory }.map(\.name)
            guard !names.isEmpty else {
                errorText = "Inbox 目录里没有文件"
                packing = false
                return
            }
            let result = try await client.packInbox(
                inboxPath: inboxPath,
                batchName: inboxBatchName,
                itemNames: names
            )
            message = "已打包 \(result.movedCount) 个文件到 \(result.targetBatchPath)"
            await discoverBatches()
        } catch {
            errorText = error.localizedDescription
        }
        packing = false
    }

    // MARK: - 推断

    private func root(for path: String) -> LibraryRoot? {
        roots
            .filter { path == $0.remotePath || path.hasPrefix($0.remotePath + "/") }
            .sorted { $0.remotePath.count > $1.remotePath.count }
            .first
    }

    private var authorIndex: [String: LibraryAuthor] {
        var index: [String: LibraryAuthor] = [:]
        for author in authors {
            index[author.name.lowercased()] = author
            for alias in author.aliases {
                let key = alias.lowercased()
                if !key.isEmpty { index[key] = author }
            }
        }
        return index
    }

    private func suggest(for name: String) -> (dir: String?, author: String?) {
        guard !vaultRoot.isEmpty else { return (nil, nil) }
        let index = authorIndex
        for token in pmlBracketTokens(name) {
            if let author = index[token.lowercased()] {
                return ("\(vaultRoot)/【\(author.name)】", author.name)
            }
        }
        let lower = name.lowercased()
        for author in authors.sorted(by: { $0.name.count > $1.name.count }) where author.name.count >= 2 {
            if lower.contains(author.name.lowercased()) {
                return ("\(vaultRoot)/【\(author.name)】", author.name)
            }
        }
        for category in categories.sorted(by: { $0.name.count > $1.name.count }) where category.name.count >= 2 {
            if lower.contains(category.name.lowercased()) {
                return ("\(vaultRoot)/\(category.name)", nil)
            }
        }
        return (vaultRoot, nil)
    }
}

// MARK: - 命名工具

func pmlIsPendingFolder(_ name: String) -> Bool {
    let trimmed = name.trimmingCharacters(in: .whitespaces)
    guard !trimmed.isEmpty else { return false }
    let patterns = [
        "^【Pending[^【】]*】$",
        "^\\[Pending[^\\[\\]]*\\]$",
        "^Pending[0-9A-Za-z_\\-.]*$",
        "^【待收纳[^【】]*】$",
        "^待收纳.*$",
    ]
    return patterns.contains {
        trimmed.range(of: $0, options: [.regularExpression, .caseInsensitive]) != nil
    }
}

func pmlBracketTokens(_ name: String) -> [String] {
    let patterns = [
        "【([^】]+)】",
        "\\[([^\\]]+)\\]",
        "（([^）]+)）",
        "\\(([^)]+)\\)",
    ]
    let ns = name as NSString
    var tokens: [String] = []
    for pattern in patterns {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { continue }
        for match in regex.matches(in: name, range: NSRange(location: 0, length: ns.length)) {
            if match.numberOfRanges > 1 {
                let token = ns.substring(with: match.range(at: 1)).trimmingCharacters(in: .whitespaces)
                if !token.isEmpty { tokens.append(token) }
            }
        }
    }
    return tokens
}
