import SwiftUI
import UIKit

private enum TriageViewMode {
    case pipeline
    case list
}

private struct PlayableVideoTarget: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
}

/// 待收纳整理：支持 Slidebox 风格单件卡片极速流水线归档与清单批量归档
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
    @State private var authorDirectoryMap: [String: String] = [:]

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

    // 交互与呈现模式
    @State private var viewMode: TriageViewMode = .pipeline
    @State private var authorSearchText = ""
    @State private var activeVideoTarget: PlayableVideoTarget?
    @State private var showNewAuthorDialog = false
    @State private var newAuthorName = ""
    @State private var hudToast: String?

    private var currentItem: PendingItem? {
        items.indices.contains(currentIndex) ? items[currentIndex] : nil
    }

    private var filteredAuthors: [LibraryAuthor] {
        let list: [LibraryAuthor]
        let query = authorSearchText.trimmingCharacters(in: .whitespacesAndNewlines)
        if query.isEmpty {
            list = authors
        } else {
            list = authors.filter { author in
                author.name.localizedCaseInsensitiveContains(query) ||
                author.aliases.contains { $0.localizedCaseInsensitiveContains(query) }
            }
        }

        // 若当前项有智能建议作者，将建议作者置顶
        if let suggested = currentItem?.suggestedAuthor?.lowercased() {
            return list.sorted { a, b in
                let aMatch = (a.name.lowercased() == suggested || a.aliases.contains { $0.lowercased() == suggested })
                let bMatch = (b.name.lowercased() == suggested || b.aliases.contains { $0.lowercased() == suggested })
                if aMatch != bMatch { return aMatch }
                return a.mediaCount > b.mediaCount
            }
        }
        return list.sorted { $0.mediaCount > $1.mediaCount }
    }

    var body: some View {
        ZStack {
            Group {
                if loading && items.isEmpty {
                    VStack(spacing: 14) {
                        ProgressView()
                        Text("正在扫描待整理目录…")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if items.isEmpty {
                    emptyCelebrationView
                } else if viewMode == .pipeline {
                    pipelineView
                } else {
                    listView
                }
            }

            if let hudToast {
                VStack {
                    HStack(spacing: 8) {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        Text(hudToast)
                            .font(.subheadline.weight(.medium))
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(.ultraThinMaterial, in: Capsule())
                    .shadow(color: .black.opacity(0.18), radius: 10, y: 5)
                    .padding(.top, 16)
                    Spacer()
                }
                .transition(.move(edge: .top).combined(with: .opacity))
                .animation(.spring(response: 0.35, dampingFraction: 0.75), value: hudToast)
            }
        }
        .navigationTitle("待整理")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                if batches.count > 1 {
                    Menu {
                        ForEach(batches) { batch in
                            Button {
                                Task { await select(batch) }
                            } label: {
                                HStack {
                                    Text(batch.name)
                                    if selectedBatch?.id == batch.id {
                                        Image(systemName: "checkmark")
                                    }
                                }
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Text(selectedBatch?.name ?? "批次")
                                .font(.caption.weight(.semibold))
                                .lineLimit(1)
                            Image(systemName: "chevron.down")
                                .font(.caption2)
                        }
                    }
                }
            }

            ToolbarItemGroup(placement: .topBarTrailing) {
                Button {
                    viewMode = (viewMode == .pipeline ? .list : .pipeline)
                } label: {
                    Label(
                        viewMode == .pipeline ? "列表模式" : "流水线",
                        systemImage: viewMode == .pipeline ? "list.bullet" : "sparkles.rectangle.stack"
                    )
                }
                .accessibilityLabel("切换整理视图")

                Button {
                    Task { await reloadAll() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .disabled(loading || busy)
            }
        }
        .task { await reloadAll() }
        .sheet(isPresented: Binding(
            get: { editingIndex != nil },
            set: { if !$0 { editingIndex = nil } }
        )) {
            targetPicker
        }
        .fullScreenCover(item: $activeVideoTarget) { target in
            GestureVideoPlayerSheet(url: target.url, title: target.title)
                .ignoresSafeArea()
        }
        .alert("新建创作者文件夹", isPresented: $showNewAuthorDialog) {
            TextField("创作者姓名（例如：三上悠亚）", text: $newAuthorName)
            Button("创建并移入") {
                let name = newAuthorName.trimmingCharacters(in: .whitespacesAndNewlines)
                newAuthorName = ""
                guard !name.isEmpty else { return }
                Task { await moveCurrentToCustomAuthor(name) }
            }
            Button("取消", role: .cancel) {
                newAuthorName = ""
            }
        } message: {
            Text("将在已整理库建立【创作者】文件夹，并将当前文件直接移入。")
        }
    }

    // MARK: - 空状态庆祝视图

    private var emptyCelebrationView: some View {
        ScrollView {
            VStack(spacing: 20) {
                Spacer().frame(height: 50)
                ZStack {
                    Circle()
                        .fill(Color.accentColor.opacity(0.12))
                        .frame(width: 100, height: 100)
                    Image(systemName: "checkmark.seal.fill")
                        .font(.system(size: 52))
                        .foregroundStyle(Color.accentColor)
                }

                Text("待整理库已全部清空！")
                    .font(.title3.weight(.bold))

                Text("当前待整理目录（\(pendingPath.isEmpty ? "未配置" : pendingPath)）下暂无需要收纳的新文件。")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 32)

                if !pendingPath.isEmpty {
                    Button {
                        Task { await reloadAll() }
                    } label: {
                        Label("重新检查新文件", systemImage: "arrow.clockwise")
                            .font(.subheadline.weight(.semibold))
                    }
                    .buttonStyle(.borderedProminent)
                    .padding(.top, 10)
                } else {
                    NavigationLink {
                        MediaSourcesView()
                    } label: {
                        Label("前往配置媒体源待整理目录", systemImage: "gearshape")
                            .font(.subheadline.weight(.semibold))
                    }
                    .buttonStyle(.borderedProminent)
                    .padding(.top, 10)
                }

                if !moveSummary.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("本次已归档记录 (\(moveSummary.count))")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)

                        ForEach(moveSummary.prefix(10), id: \.self) { item in
                            Text(item)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(14)
                    .pmlGlassCard(cornerRadius: 16)
                    .padding(.horizontal, 20)
                    .padding(.top, 20)
                }

                Spacer().frame(height: 50)
            }
        }
    }

    // MARK: - 流水线 Slidebox 归档视图

    private var pipelineView: some View {
        ScrollView {
            VStack(spacing: 14) {
                // 顶部状态指示栏
                HStack {
                    Label("还剩 \(items.count) 个待整理", systemImage: "tray.full.fill")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Spacer()
                    if processedCount > 0 {
                        Text("已收纳 \(processedCount) 个")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 20)
                .padding(.top, 8)

                if let item = currentItem {
                    // 主卡片：媒体信息与全屏点播预览
                    pipelineCard(for: item)
                        .padding(.horizontal, 16)

                    // 快速归档抽屉：创作者胶囊与搜索
                    authorDrawer(for: item)
                        .padding(.horizontal, 16)

                    // 底部辅助动作：跳过、自选、新建
                    auxiliaryActions(for: item)
                        .padding(.horizontal, 16)
                        .padding(.bottom, 24)
                }
            }
        }
    }

    private func pipelineCard(for item: PendingItem) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            // 标题与标签
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .top) {
                    Text(item.entry.name)
                        .font(.headline)
                        .lineLimit(3)
                    Spacer()
                    Text("\(currentIndex + 1)/\(items.count)")
                        .font(.caption.weight(.bold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(Color(.systemFill), in: Capsule())
                }

                HStack(spacing: 8) {
                    Label(item.entry.isDirectory ? "文件夹" : (item.entry.isVideo ? "视频" : "文件"),
                          systemImage: item.entry.isDirectory ? "folder.fill" : (item.entry.isVideo ? "video.fill" : "doc.fill"))
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.secondary)

                    Text("·")
                        .foregroundStyle(.secondary)

                    Text(pmlFormatBytes(UInt64(max(0, item.entry.sizeBytes))))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            // 预览大窗（支持点击视频直调手势播放器试看）
            ZStack {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(Color(.secondarySystemFill))

                if item.entry.isVideo {
                    videoPreviewBox(for: item)
                } else if item.entry.isImage, let previewURL {
                    CachedImageView(url: previewURL, contentMode: .fill, minHeight: 200)
                } else {
                    VStack(spacing: 8) {
                        Image(systemName: item.entry.isDirectory ? "folder.fill" : "doc.fill")
                            .font(.system(size: 46))
                            .foregroundStyle(.tint)
                        Text(item.entry.isDirectory ? "文件夹（包含子资源）" : "常规文件")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .frame(height: 210)
            .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))

            // 智能推荐条幅（若识别到创作者）
            if let suggested = item.suggestedAuthor, !suggested.isEmpty {
                HStack {
                    Image(systemName: "sparkles")
                        .foregroundStyle(.yellow)
                    Text("智能识别推荐：")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text("【\(suggested)】")
                        .font(.caption.weight(.bold))
                        .foregroundStyle(Color.accentColor)
                    Spacer()
                    Button("一键移入") {
                        Task { await moveCurrentToCustomAuthor(suggested) }
                    }
                    .font(.caption.weight(.semibold))
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(busy)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
            }
        }
        .padding(16)
        .pmlGlassCard(cornerRadius: 20)
        .task(id: item.entry.path) {
            await loadPreview()
        }
    }

    private func videoPreviewBox(for item: PendingItem) -> some View {
        Button {
            Task { await playCurrentItem() }
        } label: {
            ZStack {
                let thumbId = item.entry.assetId ?? item.entry.coverAssetId
                AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                    ZStack {
                        Color.black.opacity(0.6)
                        Image(systemName: "video.fill")
                            .font(.system(size: 40))
                            .foregroundStyle(.white.opacity(0.4))
                    }
                }

                // 悬浮居中试看胶囊
                HStack(spacing: 6) {
                    Image(systemName: "play.fill")
                        .font(.caption.weight(.bold))
                    Text("点击全屏试看")
                        .font(.caption.weight(.semibold))
                }
                .foregroundStyle(.white)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(.ultraThinMaterial, in: Capsule())
                .shadow(color: .black.opacity(0.35), radius: 8, y: 3)
            }
        }
        .buttonStyle(.plain)
    }

    private func authorDrawer(for item: PendingItem) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("快速归档至创作者")
                    .font(.subheadline.weight(.semibold))
                Spacer()
                Text("点击直接物理移动")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            // 创作者快捷搜索栏
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                TextField("搜索创作者名称或别名…", text: $authorSearchText)
                    .font(.caption)
                    .textFieldStyle(.plain)
                if !authorSearchText.isEmpty {
                    Button { authorSearchText = "" } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10))

            // 创作者按钮流式布局
            ScrollView(.vertical, showsIndicators: false) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 95), spacing: 8)], spacing: 8) {
                    ForEach(filteredAuthors.prefix(24)) { author in
                        authorChipButton(author: author, for: item)
                    }
                }
                .padding(.vertical, 4)
            }
            .frame(maxHeight: 180)
        }
        .padding(16)
        .pmlGlassCard(cornerRadius: 20)
    }

    private func authorChipButton(author: LibraryAuthor, for item: PendingItem) -> some View {
        let isSuggested = (item.suggestedAuthor?.lowercased() == author.name.lowercased())
        return Button {
            Task { await quickMoveCurrent(to: author) }
        } label: {
            HStack(spacing: 4) {
                if isSuggested {
                    Image(systemName: "sparkles")
                        .font(.caption2)
                        .foregroundStyle(.yellow)
                }
                Text(author.name)
                    .font(.caption.weight(isSuggested ? .bold : .medium))
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .background(
                isSuggested ? Color.accentColor.opacity(0.18) : Color(.secondarySystemBackground),
                in: RoundedRectangle(cornerRadius: 8)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(isSuggested ? Color.accentColor : Color.clear, lineWidth: 1)
            )
            .foregroundStyle(isSuggested ? Color.accentColor : Color.primary)
        }
        .buttonStyle(.plain)
        .disabled(busy)
    }

    private func auxiliaryActions(for item: PendingItem) -> some View {
        HStack(spacing: 12) {
            Button {
                skipCurrent()
            } label: {
                Label("跳过稍后", systemImage: "forward.fill")
                    .font(.caption.weight(.medium))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
            }
            .buttonStyle(.bordered)
            .disabled(items.count <= 1 || busy)

            Button {
                showNewAuthorDialog = true
            } label: {
                Label("＋ 新建创作者", systemImage: "folder.badge.plus")
                    .font(.caption.weight(.medium))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
            }
            .buttonStyle(.bordered)
            .disabled(busy)

            Button {
                openPicker()
            } label: {
                Label("自选目录", systemImage: "folder")
                    .font(.caption.weight(.medium))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
            }
            .buttonStyle(.bordered)
            .disabled(busy)
        }
    }

    // MARK: - 清单列表模式

    private var listView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                configSummaryCard
                actionsCard

                if !errorText.isEmpty {
                    Text(errorText).font(.footnote).foregroundStyle(.red).padding(.horizontal, 16)
                }
                if !message.isEmpty {
                    Text(message).font(.footnote).foregroundStyle(.secondary).padding(.horizontal, 16)
                }

                LazyVStack(spacing: 10) {
                    ForEach(items.indices, id: \.self) { idx in
                        let item = items[idx]
                        HStack(spacing: 12) {
                            ZStack {
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .fill(Color(.secondarySystemFill))
                                let thumbId = item.entry.assetId ?? item.entry.coverAssetId
                                AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                                    Image(systemName: item.entry.isDirectory ? "folder.fill" : "doc.fill")
                                        .font(.system(size: 20))
                                        .foregroundStyle(.tint)
                                }
                            }
                            .frame(width: 50, height: 50)
                            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                            VStack(alignment: .leading, spacing: 3) {
                                Text(item.entry.name)
                                    .font(.subheadline.weight(.medium))
                                    .lineLimit(1)
                                HStack(spacing: 6) {
                                    Text(pmlFormatBytes(UInt64(max(0, item.entry.sizeBytes))))
                                    if let author = item.suggestedAuthor {
                                        Text("·")
                                        Text("【\(author)】").foregroundStyle(.tint)
                                    }
                                }
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                            }

                            Spacer()

                            if let target = item.targetDir {
                                Button("移入") {
                                    Task { await moveItemAtIndex(idx) }
                                }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                            } else {
                                Button("选目标") {
                                    editingIndex = idx
                                    openPicker()
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)
                            }
                        }
                        .padding(12)
                        .pmlGlassCard(cornerRadius: 14)
                    }
                }
                .padding(.horizontal, 16)
            }
            .padding(.vertical, 12)
        }
    }

    private var configSummaryCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("待收纳目录：\(pendingPath)")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Text("已整理目标：\(vaultRoot)")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .pmlGlassCard(cornerRadius: 14)
        .padding(.horizontal, 16)
    }

    private var actionsCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            let ready = items.filter { $0.targetDir != nil }.count
            Text("待处理 \(items.count) 项 · 已匹配建议 \(ready) 项")
                .font(.caption)
                .foregroundStyle(.secondary)

            Button("一键收纳所有已匹配建议（\(ready) 项）") {
                Task { await moveAll() }
            }
            .buttonStyle(.borderedProminent)
            .disabled(busy || ready == 0)
        }
        .padding(14)
        .pmlGlassCard(cornerRadius: 18)
        .padding(.horizontal, 16)
    }

    // MARK: - 目标选择弹窗

    private var targetPicker: some View {
        NavigationStack {
            Form {
                Section("选择分类与创作者") {
                    Picker("分类", selection: $pickerCategoryId) {
                        Text("不指定分类").tag("")
                        ForEach(categories) { category in
                            Text(category.name).tag(category.id)
                        }
                    }
                    Picker("创作者", selection: $pickerAuthorId) {
                        Text("不指定作者").tag("")
                        ForEach(authors) { author in
                            Text(author.name).tag(author.id)
                        }
                    }
                    TextField("完整目标目录", text: $pickerPath)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section {
                    Button("确认保存目标并移动") {
                        applyPickerAndMove()
                    }
                }
            }
            .navigationTitle("自选归档目标")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { editingIndex = nil }
                }
            }
        }
    }

    // MARK: - 业务逻辑与物理移动

    private func quickMoveCurrent(to author: LibraryAuthor) async {
        guard let item = currentItem else { return }
        let targetDir = resolveAuthorDirectory(for: author.name)
        let targetPath = "\(targetDir)/\(item.entry.name)"
        await performMove(item: item, targetPath: targetPath, label: author.name)
    }

    private func moveCurrentToCustomAuthor(_ authorName: String) async {
        guard let item = currentItem else { return }
        let targetDir = resolveAuthorDirectory(for: authorName)
        let targetPath = "\(targetDir)/\(item.entry.name)"
        await performMove(item: item, targetPath: targetPath, label: authorName)
    }

    private func performMove(item: PendingItem, targetPath: String, label: String) async {
        guard let client = model.apiClient else { return }
        busy = true
        defer { busy = false }
        do {
            let result = try await client.curateMoveItem(
                sourcePath: item.entry.path,
                targetPath: targetPath,
                isDirectory: item.entry.isDirectory
            )
            if result.success || result.status == "SKIPPED_EXIST" {
                UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                showHUD("✓ 已移入【\(label)】")
                processedCount += 1
                moveSummary.insert("【\(item.entry.name)】已移入【\(label)】", at: 0)

                withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
                    if items.indices.contains(currentIndex) {
                        items.remove(at: currentIndex)
                        if currentIndex >= items.count {
                            currentIndex = max(0, items.count - 1)
                        }
                    }
                }
            } else {
                errorText = result.message
            }
        } catch {
            errorText = "移动失败: \(error.localizedDescription)"
        }
    }

    private func moveItemAtIndex(_ index: Int) async {
        guard items.indices.contains(index) else { return }
        let item = items[index]
        guard let dir = item.targetDir else { return }
        let targetPath = "\(dir)/\(item.entry.name)"
        guard let client = model.apiClient else { return }
        busy = true
        defer { busy = false }
        do {
            let result = try await client.curateMoveItem(
                sourcePath: item.entry.path,
                targetPath: targetPath,
                isDirectory: item.entry.isDirectory
            )
            if result.success || result.status == "SKIPPED_EXIST" {
                UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                showHUD("✓ 已移入【\(item.suggestedAuthor ?? "目标目录")】")
                processedCount += 1
                items.remove(at: index)
                if currentIndex >= items.count { currentIndex = max(0, items.count - 1) }
            } else {
                errorText = result.message
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
            let movedSources = Set(result.results.filter { $0.success || $0.status == "SKIPPED_EXIST" }.map(\.sourcePath))
            items.removeAll { movedSources.contains($0.entry.path) }
            if currentIndex >= items.count { currentIndex = max(0, items.count - 1) }
            showHUD("已批量归档 \(result.successCount) 项")
        } catch {
            errorText = error.localizedDescription
        }
    }

    private func skipCurrent() {
        guard !items.isEmpty else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        withAnimation {
            let item = items.remove(at: currentIndex)
            items.append(item)
            if currentIndex >= items.count { currentIndex = 0 }
        }
    }

    private func playCurrentItem() async {
        guard let client = model.apiClient, let item = currentItem, item.entry.isVideo else { return }
        do {
            let streamURL = try await client.rawMediaURL(remotePath: item.entry.path)
            await MainActor.run {
                activeVideoTarget = PlayableVideoTarget(url: streamURL, title: item.entry.name)
            }
        } catch {
            errorText = "获取播放地址失败: \(error.localizedDescription)"
        }
    }

    private func showHUD(_ text: String) {
        hudToast = text
        Task {
            try? await Task.sleep(nanoseconds: 1_600_000_000)
            await MainActor.run {
                if hudToast == text { hudToast = nil }
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

    private func applyPickerAndMove() {
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
        let targetPath = "\(path)/\(items[index].entry.name)"
        let targetItem = items[index]
        editingIndex = nil
        Task {
            await performMove(item: targetItem, targetPath: targetPath, label: author?.name ?? "自定义目录")
        }
    }

    private func resolveAuthorDirectory(for authorName: String) -> String {
        let clean = authorName.trimmingCharacters(in: CharacterSet(charactersIn: "【】[]()（） ")).lowercased()
        if let existing = authorDirectoryMap[clean] {
            return existing
        }
        if let existing = authorDirectoryMap[authorName.lowercased()] {
            return existing
        }
        return "\(vaultRoot)/【\(authorName)】"
    }

    // MARK: - 数据扫描与发现

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

            // 预探测已整理根目录下已存在的创作者文件夹（方便精准匹配物理路径）
            var folderMap: [String: String] = [:]
            if let vaultRootObj = root(for: vaultRoot), !vaultRoot.isEmpty {
                if let vaultListing = try? await client.remoteChildren(rootId: vaultRootObj.id, path: vaultRoot, refresh: false) {
                    for entry in vaultListing.items where entry.isDirectory {
                        let rawName = entry.name
                        let cleanName = rawName.trimmingCharacters(in: CharacterSet(charactersIn: "【】[]()（） ")).lowercased()
                        folderMap[cleanName] = entry.path
                        folderMap[rawName.lowercased()] = entry.path
                    }
                }
            }
            authorDirectoryMap = folderMap

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
            items = []
            return
        }
        do {
            let page = try await client.remoteChildren(rootId: root.id, path: pendingPath, refresh: true)
            let visible = page.items.filter {
                !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@") && $0.name != "desktop.ini" && $0.name != "Thumbs.db"
            }

            let batchDirs = visible.filter { $0.isDirectory && pmlIsPendingFolder($0.name) }
                .sorted { $0.name.localizedCompare($1.name) == .orderedAscending }
            let directEntries = visible.filter { !$0.isDirectory || !pmlIsPendingFolder($0.name) }
                .sorted { $0.name.localizedCompare($1.name) == .orderedAscending }

            var allBatches: [RemoteBrowserEntry] = []
            if !directEntries.isEmpty {
                allBatches.append(
                    RemoteBrowserEntry(
                        path: pendingPath,
                        name: "散装文件 (\(directEntries.count))",
                        isDirectory: true,
                        sizeBytes: 0,
                        modifiedAt: nil,
                        etag: nil,
                        assetId: nil,
                        mediaType: nil,
                        thumbnailSource: nil,
                        coverAssetId: nil
                    )
                )
            }
            allBatches.append(contentsOf: batchDirs)
            batches = allBatches

            if let first = batches.first {
                await select(first, directEntries: directEntries)
            } else {
                selectedBatch = nil
                items = []
            }
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
    }

    private func select(_ batch: RemoteBrowserEntry, directEntries: [RemoteBrowserEntry]? = nil) async {
        guard let client = model.apiClient, let root = root(for: batch.path) else { return }
        selectedBatch = batch
        message = ""
        do {
            let rawList: [RemoteBrowserEntry]
            if batch.path == pendingPath {
                if let directEntries {
                    rawList = directEntries
                } else {
                    let page = try await client.remoteChildren(rootId: root.id, path: pendingPath, refresh: true)
                    rawList = page.items.filter {
                        !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@") && (!$0.isDirectory || !pmlIsPendingFolder($0.name))
                    }
                }
            } else {
                let page = try await client.remoteChildren(rootId: root.id, path: batch.path, refresh: true)
                rawList = page.items.filter {
                    !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@")
                }
            }

            items = rawList.map { entry in
                var item = PendingItem(entry: entry, targetDir: nil, suggestedAuthor: nil)
                let suggestion = suggest(for: entry.name)
                item.targetDir = suggestion.dir
                item.suggestedAuthor = suggestion.author
                return item
            }
            currentIndex = 0
            await loadPreview()
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
                return (resolveAuthorDirectory(for: author.name), author.name)
            }
        }
        let lower = name.lowercased()
        for author in authors.sorted(by: { $0.name.count > $1.name.count }) where author.name.count >= 2 {
            if lower.contains(author.name.lowercased()) {
                return (resolveAuthorDirectory(for: author.name), author.name)
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


// MARK: - 命名与括号提取工具

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
