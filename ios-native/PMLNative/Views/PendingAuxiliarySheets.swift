import SwiftUI
import UIKit

// MARK: - 1. 文件夹受控内容浏览器 Sheet（预览待整理文件夹内部视频与图片）

private struct FolderVideoTarget: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
    let fallbackURL: URL?
}

struct PendingFolderPreviewSheet: View {
    let rootId: String
    let folderPath: String
    let folderName: String

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var currentPath: String
    @State private var entries: [RemoteBrowserEntry] = []
    @State private var loading = false
    @State private var errorText = ""
    @State private var activeVideo: FolderVideoTarget?
    @State private var activeImages: ImageViewerContext?

    // 多选与删除状态
    @State private var isEditing = false
    @State private var selectedPaths: Set<String> = []
    @State private var showDeleteConfirm = false
    @State private var deleteTargetTitle = ""
    @State private var pathsToDelete: [String] = []

    // 重命名状态
    @State private var showRenameAlert = false
    @State private var renameTargetEntry: RemoteBrowserEntry?
    @State private var renameInputText = ""
    @State private var actionBusy = false
    @State private var toastMessage: String?

    init(rootId: String, folderPath: String, folderName: String) {
        self.rootId = rootId
        self.folderPath = folderPath
        self.folderName = folderName
        _currentPath = State(initialValue: folderPath)
    }

    private var breadcrumbs: [(name: String, path: String)] {
        guard currentPath.hasPrefix(folderPath) else {
            return [(folderName, folderPath)]
        }
        var crumbs: [(String, String)] = [(folderName, folderPath)]
        let suffix = String(currentPath.dropFirst(folderPath.count))
        var running = folderPath
        for seg in suffix.split(separator: "/").map(String.init) where !seg.isEmpty {
            running += "/\(seg)"
            crumbs.append((seg, running))
        }
        return crumbs
    }

    /// 智能识别广告、宣传、死链等常见垃圾文件
    private func isJunkFile(_ entry: RemoteBrowserEntry) -> Bool {
        if entry.isDirectory { return false }
        let lower = entry.name.lowercased()
        let junkExts = [".url", ".lnk", ".torrent", ".htm", ".html", ".chm"]
        if junkExts.contains(where: { lower.hasSuffix($0) }) { return true }
        if !entry.isVideo && !entry.isImage {
            let promoWords = ["扫码", "进群", "防迷路", "最新地址", "永久地址", "解压密码", "备用群", "买车看", "发布页", "菠菜", "赌博", "吃瓜", "免费看", "充值", "赞助", "公众号", "免责声明", "readme", "说明"]
            if promoWords.contains(where: { lower.contains($0) }) { return true }
            if lower.hasSuffix(".txt") && entry.sizeBytes < 4096 { return true }
        }
        return false
    }

    private var junkEntries: [RemoteBrowserEntry] {
        entries.filter { isJunkFile($0) }
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // 面包屑导航条
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(Array(breadcrumbs.enumerated()), id: \.offset) { idx, crumb in
                            if idx > 0 {
                                Image(systemName: "chevron.right")
                                    .font(.caption2)
                                    .foregroundStyle(.tertiary)
                            }
                            Button {
                                if crumb.path != currentPath {
                                    currentPath = crumb.path
                                    Task { await loadDirectory(crumb.path) }
                                }
                            } label: {
                                Text(crumb.name)
                                    .font(.caption.weight(idx == breadcrumbs.count - 1 ? .bold : .medium))
                                    .foregroundStyle(idx == breadcrumbs.count - 1 ? Color.accentColor : Color.secondary)
                                    .lineLimit(1)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                }
                .background(Color(.secondarySystemBackground))

                Divider()

                // 智能识别垃圾文件一键清理横幅（存在垃圾文件且非多选模式下浮现）
                if !isEditing && !junkEntries.isEmpty {
                    HStack(spacing: 10) {
                        Image(systemName: "sparkles")
                            .foregroundStyle(.orange)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("发现 \(junkEntries.count) 个广告/宣传垃圾文件")
                                .font(.caption.weight(.bold))
                                .foregroundStyle(.primary)
                            Text("例如：\(junkEntries.prefix(2).map(\.name).joined(separator: ", "))")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        Spacer()
                        Button {
                            pathsToDelete = junkEntries.map(\.path)
                            deleteTargetTitle = "\(junkEntries.count) 个智能识别的广告/垃圾文件"
                            showDeleteConfirm = true
                        } label: {
                            Text("一键清理")
                                .font(.caption2.weight(.bold))
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .background(Color.red, in: Capsule())
                                .foregroundStyle(.white)
                        }
                        .buttonStyle(.plain)
                        .disabled(actionBusy)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.orange.opacity(0.12))
                }

                if loading && entries.isEmpty {
                    VStack(spacing: 12) {
                        ProgressView()
                        Text("正在读取文件夹内容…")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if !errorText.isEmpty {
                    ContentUnavailableView("加载失败", systemImage: "exclamationmark.triangle", description: Text(errorText))
                } else if entries.isEmpty {
                    ContentUnavailableView("空目录", systemImage: "folder", description: Text("该目录下暂无媒体文件"))
                } else {
                    List {
                        ForEach(entries) { entry in
                            HStack(spacing: 12) {
                                if isEditing {
                                    Button {
                                        if selectedPaths.contains(entry.path) {
                                            selectedPaths.remove(entry.path)
                                        } else {
                                            selectedPaths.insert(entry.path)
                                        }
                                    } label: {
                                        Image(systemName: selectedPaths.contains(entry.path) ? "checkmark.circle.fill" : "circle")
                                            .font(.title3)
                                            .foregroundStyle(selectedPaths.contains(entry.path) ? Color.accentColor : Color.secondary)
                                    }
                                    .buttonStyle(.plain)
                                }

                                Button {
                                    if isEditing {
                                        if selectedPaths.contains(entry.path) {
                                            selectedPaths.remove(entry.path)
                                        } else {
                                            selectedPaths.insert(entry.path)
                                        }
                                    } else {
                                        handleTap(entry)
                                    }
                                } label: {
                                    HStack(spacing: 12) {
                                        ZStack {
                                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                                .fill(Color(.secondarySystemFill))
                                            if let thumbId = entry.assetId ?? entry.coverAssetId {
                                                AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                                                    Image(systemName: entry.symbol)
                                                        .foregroundStyle(.tint)
                                                }
                                            } else {
                                                Image(systemName: entry.symbol)
                                                    .font(.title3)
                                                    .foregroundStyle(entry.isDirectory ? Color.accentColor : Color.secondary)
                                            }
                                        }
                                        .frame(width: 48, height: 48)
                                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))

                                        VStack(alignment: .leading, spacing: 3) {
                                            HStack(spacing: 4) {
                                                Text(entry.name)
                                                    .font(.subheadline.weight(.medium))
                                                    .lineLimit(2)
                                                if isJunkFile(entry) {
                                                    Text("垃圾")
                                                        .font(.system(size: 9, weight: .bold))
                                                        .padding(.horizontal, 4)
                                                        .padding(.vertical, 1)
                                                        .background(Color.red.opacity(0.15), in: Capsule())
                                                        .foregroundStyle(.red)
                                                }
                                            }

                                            HStack(spacing: 6) {
                                                Text(entry.isDirectory ? "子目录" : (entry.isVideo ? "视频" : (entry.isImage ? "图片" : "文件")))
                                                if !entry.isDirectory {
                                                    Text("·")
                                                    Text(pmlFormatBytes(UInt64(max(0, entry.sizeBytes))))
                                                }
                                            }
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                        }

                                        Spacer()

                                        if !isEditing {
                                            Image(systemName: entry.isDirectory ? "chevron.right" : (entry.isVideo ? "play.circle.fill" : "eye"))
                                                .foregroundStyle(entry.isVideo ? Color.accentColor : Color.secondary)
                                        }
                                    }
                                }
                                .buttonStyle(.plain)
                            }
                            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                if !isEditing {
                                    Button(role: .destructive) {
                                        pathsToDelete = [entry.path]
                                        deleteTargetTitle = "【\(entry.name)】"
                                        showDeleteConfirm = true
                                    } label: {
                                        Label("删除", systemImage: "trash")
                                    }

                                    Button {
                                        renameTargetEntry = entry
                                        renameInputText = entry.name
                                        showRenameAlert = true
                                    } label: {
                                        Label("重命名", systemImage: "pencil")
                                    }
                                    .tint(.orange)
                                }
                            }
                            .contextMenu {
                                if !isEditing {
                                    Button {
                                        renameTargetEntry = entry
                                        renameInputText = entry.name
                                        showRenameAlert = true
                                    } label: {
                                        Label("重命名", systemImage: "pencil")
                                    }

                                    Button(role: .destructive) {
                                        pathsToDelete = [entry.path]
                                        deleteTargetTitle = "【\(entry.name)】"
                                        showDeleteConfirm = true
                                    } label: {
                                        Label("彻底删除", systemImage: "trash")
                                    }
                                }
                            }
                        }
                    }
                    .listStyle(.plain)
                }

                // 多选模式底部操作栏
                if isEditing {
                    HStack(spacing: 12) {
                        Button(selectedPaths.count == entries.count ? "取消全选" : "全选 (\(entries.count))") {
                            if selectedPaths.count == entries.count {
                                selectedPaths.removeAll()
                            } else {
                                selectedPaths = Set(entries.map(\.path))
                            }
                        }
                        .font(.subheadline.weight(.medium))

                        Spacer()

                        Button(role: .destructive) {
                            pathsToDelete = Array(selectedPaths)
                            deleteTargetTitle = "所选 \(selectedPaths.count) 个文件"
                            showDeleteConfirm = true
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "trash.fill")
                                Text("删除所选 (\(selectedPaths.count))")
                            }
                            .fontWeight(.bold)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(.red)
                        .disabled(selectedPaths.isEmpty || actionBusy)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .background(.ultraThinMaterial)
                }
            }
            .navigationTitle(isEditing ? "已选 \(selectedPaths.count) 项" : "预览文件夹内容")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    if isEditing {
                        Button("取消") {
                            isEditing = false
                            selectedPaths.removeAll()
                        }
                    } else {
                        Button("关闭") { dismiss() }
                    }
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if !entries.isEmpty {
                        Button(isEditing ? "完成" : "多选") {
                            isEditing.toggle()
                            if !isEditing { selectedPaths.removeAll() }
                        }
                        .font(.subheadline.weight(isEditing ? .bold : .regular))
                    }

                    if !isEditing {
                        Button {
                            Task { await loadDirectory(currentPath, forceRefresh: true) }
                        } label: {
                            Image(systemName: "arrow.clockwise")
                        }
                        .disabled(loading || actionBusy)
                    }
                }
            }
            .task { await loadDirectory(folderPath) }
            .fullScreenCover(item: $activeVideo) { target in
                GestureVideoPlayerSheet(
                    url: target.url,
                    title: target.title,
                    fallbackURL: target.fallbackURL,
                    sourceLabel: "文件夹预览"
                )
                .ignoresSafeArea()
            }
            .fullScreenCover(item: $activeImages) { ctx in
                ImageViewer(context: ctx)
                    .environmentObject(model)
            }
            .alert("重命名文件", isPresented: $showRenameAlert) {
                TextField("输入新文件名", text: $renameInputText)
                Button("取消", role: .cancel) { renameTargetEntry = nil }
                Button("确定重命名") {
                    if let entry = renameTargetEntry {
                        Task { await executeRename(entry: entry, newName: renameInputText) }
                    }
                }
            } message: {
                Text("原文件名：\(renameTargetEntry?.name ?? "")")
            }
            .confirmationDialog("确认删除", isPresented: $showDeleteConfirm, titleVisibility: .visible) {
                Button("确定彻底删除 \(deleteTargetTitle)", role: .destructive) {
                    Task { await executeDelete(paths: pathsToDelete) }
                }
                Button("取消", role: .cancel) { pathsToDelete.removeAll() }
            } message: {
                Text("此操作将直接从存储中彻底删除该文件，无法撤销，请谨慎操作！")
            }
            .overlay(alignment: .top) {
                if let msg = toastMessage {
                    Text(msg)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .background(.black.opacity(0.75), in: Capsule())
                        .padding(.top, 10)
                        .transition(.move(edge: .top).combined(with: .opacity))
                }
            }
        }
    }

    private func showToast(_ text: String) {
        withAnimation { toastMessage = text }
        Task {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            withAnimation { toastMessage = nil }
        }
    }

    private func executeRename(entry: RemoteBrowserEntry, newName: String) async {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != entry.name, let client = model.apiClient else { return }
        actionBusy = true
        defer { actionBusy = false }
        let parentDir = entry.path.split(separator: "/").dropLast().joined(separator: "/")
        let newTargetPath = parentDir.isEmpty ? "/\(trimmed)" : "/\(parentDir)/\(trimmed)"

        do {
            try await client.renameCurateEntry(sourcePath: entry.path, targetPath: newTargetPath, isDirectory: entry.isDirectory)
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            showToast("已重命名为 \(trimmed)")
            await loadDirectory(currentPath, forceRefresh: true)
        } catch {
            showToast("重命名失败: \(error.localizedDescription)")
        }
    }

    private func executeDelete(paths: [String]) async {
        guard !paths.isEmpty, let client = model.apiClient else { return }
        actionBusy = true
        defer { actionBusy = false }
        var successCount = 0
        var failCount = 0

        for path in paths {
            do {
                try await client.deleteCurateFile(filePath: path)
                successCount += 1
            } catch {
                failCount += 1
            }
        }

        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        if failCount == 0 {
            showToast("已成功删除 \(successCount) 项")
        } else {
            showToast("删除完成：成功 \(successCount) 项，失败 \(failCount) 项")
        }

        selectedPaths.removeAll()
        isEditing = false
        await loadDirectory(currentPath, forceRefresh: true)
    }

    private func loadDirectory(_ path: String, forceRefresh: Bool = false) async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        defer { loading = false }
        do {
            var page = try await client.remoteChildren(rootId: rootId, path: path, refresh: forceRefresh)
            if !forceRefresh && page.items.isEmpty {
                page = try await client.remoteChildren(rootId: rootId, path: path, refresh: true)
            }
            entries = page.items.filter {
                !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@") && $0.name != "desktop.ini" && $0.name != "Thumbs.db"
            }
            .sorted { a, b in
                if a.isDirectory != b.isDirectory { return a.isDirectory }
                return a.name.localizedStandardCompare(b.name) == .orderedAscending
            }
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
    }

    private func handleTap(_ entry: RemoteBrowserEntry) {
        if entry.isDirectory {
            currentPath = entry.path
            Task { await loadDirectory(entry.path) }
            return
        }
        if entry.isVideo {
            Task {
                if let source = await model.playbackSource(assetId: entry.assetId, remotePath: entry.path) {
                    activeVideo = FolderVideoTarget(url: source.url, title: entry.name, fallbackURL: source.fallbackURL)
                }
            }
            return
        }
        if entry.isImage {
            let images = entries.filter { $0.isImage }
            let startIdx = images.firstIndex(where: { $0.id == entry.id }) ?? 0
            activeImages = ImageViewerContext(entries: images, startIndex: startIdx)
        }
    }
}

// MARK: - 2. 高级分类与作者级联选择 Sheet（对齐 Web CurateDialog）

struct PendingAdvancedPickerResult {
    let targetVaultPath: String
    let author: String?
    let categoryPath: String?
    let categoryName: String?
    let newCategoryCreated: VaultCategoryNode?
    let updatedAliases: (authorId: String, aliases: [String])?
}

struct PendingAdvancedPickerSheet: View {
    let item: PendingCurationItem
    let vaultRoot: String
    let categoryTree: [VaultCategoryNode]
    let authors: [VaultAuthorEntry]
    let onConfirm: (PendingAdvancedPickerResult) -> Void

    @Environment(\.dismiss) private var dismiss

    @State private var omniQuery = ""
    @State private var selectedL1: VaultCategoryNode?
    @State private var selectedL2: VaultCategoryNode?
    @State private var selectedCategoryPath = ""
    @State private var selectedCategoryName = ""
    @State private var directCategoryMode = false
    @State private var authorNameInput = ""
    @State private var authorFilterKeyword = ""
    @State private var aliasInput = ""
    @State private var copiedTitleToast = false

    // 新建分类
    @State private var showCreateCategory = false
    @State private var newCategoryName = ""
    @State private var createAsChild = true
    @State private var localCategoryTree: [VaultCategoryNode] = []
    @State private var createdCategoryNode: VaultCategoryNode?

    // 最近常用分类（持久化 UserDefaults）
    @AppStorage("pml.pending.recentCategories") private var recentCategoriesRaw = ""

    private var recentCategoryPaths: [String] {
        recentCategoriesRaw.split(separator: "|").map(String.init).filter { !$0.isEmpty }
    }

    private var flatCategories: [VaultCategoryNode] {
        MediaOrganizer.flattenCategories(localCategoryTree)
    }

    private var omniMatchedCategories: [VaultCategoryNode] {
        let q = omniQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return [] }
        return flatCategories.filter {
            $0.cleanName.localizedCaseInsensitiveContains(q) || $0.fullDisplayName.localizedCaseInsensitiveContains(q)
        }.prefix(8).map { $0 }
    }

    private var omniMatchedAuthors: [VaultAuthorEntry] {
        let q = omniQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return [] }
        return authors.filter { a in
            a.canonicalName.localizedCaseInsensitiveContains(q) ||
            a.aliases.contains { $0.localizedCaseInsensitiveContains(q) }
        }
        .sorted { ($0.vaultPath != nil) && ($1.vaultPath == nil) }
        .prefix(8)
        .map { $0 }
    }

    private var authorsInSelectedCategory: [VaultAuthorEntry] {
        guard !selectedCategoryPath.isEmpty else { return [] }
        let base = authors.filter { $0.categoryPath == selectedCategoryPath }
        let q = authorFilterKeyword.trimmingCharacters(in: .whitespacesAndNewlines)
        if q.isEmpty { return base }
        return base.filter { $0.canonicalName.localizedCaseInsensitiveContains(q) }
    }

    private var matchedExistingAuthor: VaultAuthorEntry? {
        let clean = authorNameInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))
        guard !clean.isEmpty else { return nil }
        return authors.first { $0.canonicalName.lowercased() == clean.lowercased() }
    }

    private var computedDestination: String? {
        let catPath = selectedCategoryPath.trimmingCharacters(in: .whitespacesAndNewlines)
        if directCategoryMode {
            return catPath.isEmpty ? nil : catPath
        }
        let authorClean = authorNameInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))
        if authorClean.isEmpty {
            return nil
        }
        if !catPath.isEmpty {
            return "\(catPath)/【\(authorClean)】"
        }
        if let existing = matchedExistingAuthor, let vp = existing.vaultPath, !vp.isEmpty {
            return vp
        }
        return vaultRoot.isEmpty ? nil : "\(vaultRoot)/【\(authorClean)】"
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    // 当前项目名（支持长按自由选词部分复制、整条复制与分段点选）
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text("当前归类项目")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            Spacer()
                            Button {
                                UIPasteboard.general.string = item.entry.name
                                copiedTitleToast = true
                                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                Task {
                                    try? await Task.sleep(nanoseconds: 1_800_000_000)
                                    copiedTitleToast = false
                                }
                            } label: {
                                Label(copiedTitleToast ? "已复制" : "复制标题", systemImage: copiedTitleToast ? "checkmark" : "doc.on.doc")
                                    .font(.caption2.weight(.medium))
                                    .foregroundStyle(copiedTitleToast ? .green : .accentColor)
                            }
                            .buttonStyle(.plain)
                        }

                        Text(item.entry.name)
                            .font(.subheadline.weight(.semibold))
                            .textSelection(.enabled)
                            .lineLimit(nil)
                            .fixedSize(horizontal: false, vertical: true)

                        if !item.candidateAuthorChips.isEmpty {
                            VStack(alignment: .leading, spacing: 5) {
                                Text("智能分段候选（点击直接填入作者）：")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)

                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 6) {
                                        ForEach(item.candidateAuthorChips, id: \.self) { chip in
                                            let isSelected = authorNameInput.lowercased() == chip.lowercased()
                                            Button {
                                                authorNameInput = chip
                                                directCategoryMode = false
                                                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                            } label: {
                                                HStack(spacing: 3) {
                                                    if isSelected {
                                                        Image(systemName: "checkmark")
                                                            .font(.system(size: 9, weight: .bold))
                                                    }
                                                    Text(chip)
                                                        .font(.caption2.weight(.medium))
                                                }
                                                .padding(.horizontal, 9)
                                                .padding(.vertical, 4)
                                                .background(isSelected ? Color.purple : Color.purple.opacity(0.12), in: Capsule())
                                                .foregroundStyle(isSelected ? .white : .purple)
                                            }
                                            .buttonStyle(.plain)
                                        }
                                    }
                                }
                            }
                            .padding(.top, 4)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))

                    // 全能搜索栏（分类 + 已有作者）
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 8) {
                            Image(systemName: "magnifyingglass")
                                .foregroundStyle(.secondary)
                            TextField("⚡ 搜分类或已有作者（选作者自动带出分类）…", text: $omniQuery)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                            if !omniQuery.isEmpty {
                                Button { omniQuery = "" } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                        .padding(10)
                        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 10))

                        if !omniQuery.trimmingCharacters(in: .whitespaces).isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                if !omniMatchedCategories.isEmpty {
                                    Text("匹配分类")
                                        .font(.caption2.weight(.semibold))
                                        .foregroundStyle(.secondary)
                                    ForEach(omniMatchedCategories) { cat in
                                        Button {
                                            applySelectedCategory(cat)
                                            omniQuery = ""
                                        } label: {
                                            HStack {
                                                Image(systemName: "folder.fill").foregroundStyle(.green)
                                                Text(cat.fullDisplayName).font(.caption.weight(.medium))
                                                Spacer()
                                            }
                                            .padding(.vertical, 4)
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                                if !omniMatchedAuthors.isEmpty {
                                    Text("匹配已有作者（点击自动带出分类）")
                                        .font(.caption2.weight(.semibold))
                                        .foregroundStyle(.secondary)
                                    ForEach(omniMatchedAuthors) { auth in
                                        Button {
                                            applySelectedAuthor(auth)
                                            omniQuery = ""
                                        } label: {
                                            HStack {
                                                Image(systemName: "person.fill").foregroundStyle(.purple)
                                                VStack(alignment: .leading, spacing: 2) {
                                                    Text("【\(auth.canonicalName)】").font(.caption.weight(.bold))
                                                    if let vp = auth.vaultPath {
                                                        Text(vp).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                                                    }
                                                }
                                                Spacer()
                                            }
                                            .padding(.vertical, 4)
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                                if omniMatchedCategories.isEmpty && omniMatchedAuthors.isEmpty {
                                    Text("未找到已有分类或作者，可在下方直接新建")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            .padding(10)
                            .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 10))
                        }
                    }

                    // 最近常用分类气泡
                    if !recentCategoryPaths.isEmpty {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("常用分类")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 8) {
                                    ForEach(recentCategoryPaths, id: \.self) { path in
                                        let leaf = MediaOrganizer.cleanCategoryName(path.split(separator: "/").last.map(String.init) ?? path)
                                        let parent = path.split(separator: "/").dropLast().last.map { MediaOrganizer.cleanCategoryName(String($0)) }
                                        let active = selectedCategoryPath == path
                                        Button {
                                            selectCategoryByPath(path)
                                        } label: {
                                            HStack(spacing: 3) {
                                                Text(leaf)
                                                    .font(.caption.weight(active ? .bold : .medium))
                                                if let p = parent, !p.isEmpty {
                                                    Text("(\(p))")
                                                        .font(.system(size: 9))
                                                        .foregroundStyle(active ? Color.green.opacity(0.85) : .secondary)
                                                }
                                            }
                                            .padding(.horizontal, 10)
                                            .padding(.vertical, 6)
                                            .background(active ? Color.green.opacity(0.2) : Color(.secondarySystemBackground), in: Capsule())
                                            .overlay(Capsule().stroke(active ? Color.green : Color.clear, lineWidth: 1))
                                            .foregroundStyle(active ? Color.green : Color.primary)
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
                            }
                        }
                    }

                    // 三级平铺级联分类胶囊 + 新建分类
                    VStack(alignment: .leading, spacing: 10) {
                        HStack {
                            Text("选择目标分类（三级级联）")
                                .font(.subheadline.weight(.semibold))
                            Spacer()
                            Button {
                                withAnimation { showCreateCategory.toggle() }
                            } label: {
                                Label("新建分类", systemImage: "folder.badge.plus")
                                    .font(.caption.weight(.semibold))
                            }
                        }

                        if showCreateCategory {
                            VStack(alignment: .leading, spacing: 8) {
                                TextField("输入新分类名（如：4K经典 / 推特）", text: $newCategoryName)
                                    .textFieldStyle(.roundedBorder)
                                if !selectedCategoryPath.isEmpty {
                                    Toggle("在已选分类下创建子目录", isOn: $createAsChild)
                                        .font(.caption)
                                }
                                HStack {
                                    Spacer()
                                    Button("取消") { showCreateCategory = false }
                                        .font(.caption)
                                    Button("确认添加") { createInlineCategory() }
                                        .font(.caption.weight(.semibold))
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.small)
                                        .disabled(newCategoryName.trimmingCharacters(in: .whitespaces).isEmpty)
                                }
                            }
                            .padding(10)
                            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 10))
                        }

                        // 一级大类
                        cascaderRow(title: "一级大类", nodes: localCategoryTree, activeId: selectedL1?.id) { node in
                            selectedL1 = node
                            selectedL2 = nil
                            selectedCategoryPath = node.path
                            selectedCategoryName = node.fullDisplayName
                        }

                        // 二级子分类
                        if let l1 = selectedL1, !l1.children.isEmpty {
                            cascaderRow(title: "二级子分类", nodes: l1.children, activeId: selectedL2?.id) { node in
                                selectedL2 = node
                                selectedCategoryPath = node.path
                                selectedCategoryName = node.fullDisplayName
                            }
                        }

                        // 三级分类
                        if let l2 = selectedL2, !l2.children.isEmpty {
                            cascaderRow(title: "三级分类", nodes: l2.children, activeId: flatCategories.first(where: { $0.path == selectedCategoryPath })?.id) { node in
                                selectedCategoryPath = node.path
                                selectedCategoryName = node.fullDisplayName
                            }
                        }
                    }

                    // 收纳模式切换：归入作者子目录 vs 直接归入分类根目录
                    Picker("收纳方式", selection: $directCategoryMode) {
                        Text("归入作者子目录").tag(false)
                        Text("直接归入分类根目录").tag(true)
                    }
                    .pickerStyle(.segmented)

                    // 作者配置面板
                    if !directCategoryMode {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("作者目录名（生成【作者】文件夹）")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)

                            HStack {
                                Image(systemName: "person.fill").foregroundStyle(.purple)
                                TextField("输入新作者名，或点选下方已有作者…", text: $authorNameInput)
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                                if !authorNameInput.isEmpty {
                                    Button { authorNameInput = "" } label: {
                                        Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                                    }
                                }
                                Button {
                                    if let clip = UIPasteboard.general.string?.trimmingCharacters(in: .whitespacesAndNewlines), !clip.isEmpty {
                                        authorNameInput = clip
                                        directCategoryMode = false
                                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                    }
                                } label: {
                                    Image(systemName: "doc.on.clipboard")
                                        .font(.subheadline)
                                        .foregroundStyle(.purple)
                                }
                            }
                            .padding(10)
                            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 10))

                            // 文件名候选词快捷填入
                            if !item.candidateAuthorChips.isEmpty {
                                ScrollView(.horizontal, showsIndicators: false) {
                                    HStack(spacing: 8) {
                                        Text("文件名候选:")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                        ForEach(item.candidateAuthorChips, id: \.self) { chip in
                                            Button {
                                                authorNameInput = chip
                                            } label: {
                                                Text(chip)
                                                    .font(.caption.weight(.medium))
                                                    .padding(.horizontal, 10)
                                                    .padding(.vertical, 5)
                                                    .background(Color.purple.opacity(0.14), in: Capsule())
                                                    .foregroundStyle(.purple)
                                            }
                                            .buttonStyle(.plain)
                                        }
                                    }
                                }
                            }

                            // 所选分类下已有作者
                            if !selectedCategoryPath.isEmpty {
                                VStack(alignment: .leading, spacing: 6) {
                                    HStack {
                                        Text("该分类下已有作者 (\(authorsInSelectedCategory.count))")
                                            .font(.caption.weight(.semibold))
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                    }
                                    if authorsInSelectedCategory.count > 6 {
                                        TextField("过滤该分类作者…", text: $authorFilterKeyword)
                                            .font(.caption)
                                            .textFieldStyle(.roundedBorder)
                                    }
                                    if authorsInSelectedCategory.isEmpty {
                                        Text("该分类下暂无已有作者，上方直接输入即可新建")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    } else {
                                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 90), spacing: 8)], spacing: 8) {
                                            ForEach(authorsInSelectedCategory.prefix(24)) { auth in
                                                let active = authorNameInput.lowercased() == auth.canonicalName.lowercased()
                                                Button {
                                                    authorNameInput = auth.canonicalName
                                                } label: {
                                                    Text(auth.canonicalName)
                                                        .font(.caption.weight(active ? .bold : .medium))
                                                        .lineLimit(1)
                                                        .frame(maxWidth: .infinity)
                                                        .padding(.horizontal, 8)
                                                        .padding(.vertical, 7)
                                                        .background(active ? Color.purple.opacity(0.2) : Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 8))
                                                        .overlay(RoundedRectangle(cornerRadius: 8).stroke(active ? Color.purple : Color.clear, lineWidth: 1))
                                                        .foregroundStyle(active ? Color.purple : Color.primary)
                                                }
                                                .buttonStyle(.plain)
                                            }
                                        }
                                    }
                                }
                            }

                            // 补充作者别名（选填）
                            if matchedExistingAuthor != nil {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text("补充作者别名（选填，逗号分隔，下次自动识别）")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                    TextField("例如：英文名, 简称", text: $aliasInput)
                                        .font(.caption)
                                        .textFieldStyle(.roundedBorder)
                                }
                            }
                        }
                    }

                    // 最终目标路径预览
                    VStack(alignment: .leading, spacing: 4) {
                        Label("目标路径预览", systemImage: "safari")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text(computedDestination ?? "请先选择目标分类或输入作者名")
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(computedDestination != nil ? Color.accentColor : Color.secondary)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                    .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                }
                .padding(16)
            }
            .navigationTitle("自定义分类与作者")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("保存至已归纳") {
                        commitSelection()
                    }
                    .fontWeight(.bold)
                    .disabled(computedDestination == nil)
                }
            }
            .onAppear {
                localCategoryTree = categoryTree
                if let matched = item.matchedAuthor, !matched.isEmpty {
                    authorNameInput = matched
                } else if let firstChip = item.candidateAuthorChips.first, !firstChip.isEmpty {
                    authorNameInput = firstChip
                } else {
                    authorNameInput = ""
                }
                if let cp = item.matchedCategoryPath, !cp.isEmpty {
                    selectCategoryByPath(cp)
                }
                if item.isAssigned && (item.matchedAuthor == nil || item.matchedAuthor?.isEmpty == true) {
                    directCategoryMode = true
                }
            }
        }
    }

    private func cascaderRow(
        title: String,
        nodes: [VaultCategoryNode],
        activeId: String?,
        onTap: @escaping (VaultCategoryNode) -> Void
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(nodes) { node in
                        let isActive = activeId == node.id || selectedCategoryPath == node.path
                        Button {
                            onTap(node)
                        } label: {
                            HStack(spacing: 4) {
                                Text(node.cleanName)
                                    .font(.caption.weight(isActive ? .bold : .medium))
                                if !node.children.isEmpty {
                                    Text("\(node.children.count)")
                                        .font(.system(size: 10, weight: .semibold))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1)
                                        .background(Color.primary.opacity(0.08), in: Capsule())
                                }
                            }
                            .padding(.horizontal, 12)
                            .padding(.vertical, 7)
                            .background(isActive ? Color.accentColor.opacity(0.18) : Color(.secondarySystemBackground), in: Capsule())
                            .overlay(Capsule().stroke(isActive ? Color.accentColor : Color.clear, lineWidth: 1))
                            .foregroundStyle(isActive ? Color.accentColor : Color.primary)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private func applySelectedCategory(_ cat: VaultCategoryNode) {
        selectCategoryByPath(cat.path)
    }

    private func applySelectedAuthor(_ auth: VaultAuthorEntry) {
        directCategoryMode = false
        authorNameInput = auth.canonicalName
        if let cp = auth.categoryPath, !cp.isEmpty {
            selectCategoryByPath(cp)
        }
    }

    private func selectCategoryByPath(_ path: String) {
        selectedCategoryPath = path
        if let hit = flatCategories.first(where: { $0.path == path }) {
            selectedCategoryName = hit.fullDisplayName
        } else {
            selectedCategoryName = path.split(separator: "/").suffix(2).map { MediaOrganizer.cleanCategoryName(String($0)) }.joined(separator: " / ")
        }
        // 同步级联高亮
        for l1 in localCategoryTree {
            if l1.path == path {
                selectedL1 = l1
                selectedL2 = nil
                return
            }
            for l2 in l1.children {
                if l2.path == path {
                    selectedL1 = l1
                    selectedL2 = l2
                    return
                }
                for l3 in l2.children where l3.path == path {
                    selectedL1 = l1
                    selectedL2 = l2
                    return
                }
            }
        }
    }

    private func createInlineCategory() {
        let trimmed = newCategoryName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !vaultRoot.isEmpty else { return }
        let segment = (trimmed.hasPrefix("{") && trimmed.hasSuffix("}")) ? trimmed : "{\(trimmed)}"
        let parentPath = (createAsChild && !selectedCategoryPath.isEmpty) ? selectedCategoryPath : vaultRoot
        let fullPath = "\(parentPath)/\(segment)"
        let clean = MediaOrganizer.cleanCategoryName(trimmed)
        let node = VaultCategoryNode(
            id: "custom-\(fullPath)",
            rawName: trimmed,
            cleanName: clean,
            fullDisplayName: (createAsChild && !selectedCategoryName.isEmpty) ? "\(selectedCategoryName) / \(clean)" : clean,
            path: fullPath,
            parentId: nil,
            children: []
        )
        if !createAsChild || selectedCategoryPath.isEmpty {
            localCategoryTree.append(node)
            selectedL1 = node
            selectedL2 = nil
        }
        selectedCategoryPath = node.path
        selectedCategoryName = node.fullDisplayName
        createdCategoryNode = node
        newCategoryName = ""
        showCreateCategory = false
    }

    private func recordRecentCategory(_ path: String) {
        guard !path.isEmpty else { return }
        var list = recentCategoryPaths.filter { $0 != path }
        list.insert(path, at: 0)
        recentCategoriesRaw = Array(list.prefix(6)).joined(separator: "|")
    }

    private func commitSelection() {
        guard let dest = computedDestination else { return }
        if !selectedCategoryPath.isEmpty {
            recordRecentCategory(selectedCategoryPath)
        }
        let authorClean = directCategoryMode ? nil : {
            let s = authorNameInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))
            return s.isEmpty ? nil : s
        }()

        var aliasUpdate: (String, [String])? = nil
        let rawAliases = aliasInput
            .components(separatedBy: CharacterSet(charactersIn: ",，"))
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        if !rawAliases.isEmpty, let existing = matchedExistingAuthor {
            let merged = Array(Set(existing.aliases + rawAliases))
            aliasUpdate = (existing.id, merged)
        }

        if !selectedCategoryPath.isEmpty {
            var list = recentCategoryPaths.filter { $0 != selectedCategoryPath }
            list.insert(selectedCategoryPath, at: 0)
            if list.count > 10 {
                list = Array(list.prefix(10))
            }
            recentCategoriesRaw = list.joined(separator: "|")
        }

        onConfirm(
            PendingAdvancedPickerResult(
                targetVaultPath: dest,
                author: authorClean,
                categoryPath: selectedCategoryPath.isEmpty ? nil : selectedCategoryPath,
                categoryName: selectedCategoryName.isEmpty ? nil : selectedCategoryName,
                newCategoryCreated: createdCategoryNode,
                updatedAliases: aliasUpdate
            )
        )
        dismiss()
    }
}

// MARK: - 3. 后台移动任务监控中心 Sheet

struct PendingJobsMonitorSheet: View {
    let jobs: [CurateMoveJobSnapshot]
    let onRefresh: () async -> Void
    let onCancelJob: (String) async -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if jobs.isEmpty {
                    ContentUnavailableView("暂无后台移动任务", systemImage: "tray.and.arrow.up", description: Text("在已归纳清单点击“一键后台移动”后，服务器会在后台串行防风控执行。"))
                } else {
                    List(jobs) { job in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Text(job.projectName)
                                    .font(.headline)
                                Spacer()
                                statusBadge(for: job)
                            }

                            ProgressView(value: Double(job.processed), total: Double(max(1, job.total)))
                                .tint(job.status == "aborted" ? .red : (job.status == "completed" ? .green : .accentColor))

                            HStack(spacing: 12) {
                                Text("进度 \(job.processed)/\(job.total)")
                                Text("成功 \(job.successCount)").foregroundStyle(.green)
                                if job.skippedCount > 0 {
                                    Text("跳过 \(job.skippedCount)").foregroundStyle(.orange)
                                }
                                if job.failedCount > 0 {
                                    Text("失败 \(job.failedCount)").foregroundStyle(.red)
                                }
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)

                            if let current = job.currentItem, job.isRunning {
                                Text("正在移动：\(current)")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }

                            if let reason = job.abortReason {
                                Text(reason)
                                    .font(.caption2)
                                    .foregroundStyle(.red)
                            }

                            if !job.errors.isEmpty {
                                VStack(alignment: .leading, spacing: 2) {
                                    ForEach(job.errors.prefix(5), id: \.self) { err in
                                        Text("• \(err)")
                                            .font(.caption2)
                                            .foregroundStyle(.red)
                                            .lineLimit(2)
                                    }
                                }
                            }

                            if job.isRunning {
                                HStack {
                                    Label("服务端串行防风控执行中（可锁屏/切后台）", systemImage: "shield.checkered")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    Button("紧急停止", role: .destructive) {
                                        Task { await onCancelJob(job.id) }
                                    }
                                    .font(.caption.weight(.semibold))
                                    .buttonStyle(.bordered)
                                    .controlSize(.small)
                                }
                            }
                        }
                        .padding(.vertical, 6)
                    }
                    .listStyle(.insetGrouped)
                }
            }
            .navigationTitle("服务器后台移动任务")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("关闭") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task { await onRefresh() }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                }
            }
        }
    }

    private func statusBadge(for job: CurateMoveJobSnapshot) -> some View {
        let (text, color): (String, Color) = {
            switch job.status {
            case "running": return ("执行中", .blue)
            case "completed": return ("已完成", .green)
            case "cancelled": return ("已手动终止", .orange)
            case "aborted": return ("熔断中止", .red)
            default: return (job.status, .secondary)
            }
        }()
        return Text(text)
            .font(.caption2.weight(.bold))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(color.opacity(0.15), in: Capsule())
            .foregroundStyle(color)
    }
}

// MARK: - 4. 服务器过滤词管理 Sheet

struct PendingFilterWordsSheet: View {
    @Binding var filterWords: [String]
    let onSave: ([String]) async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var newWord = ""
    @State private var saving = false

    var body: some View {
        NavigationStack {
            List {
                Section("新增自定义过滤词") {
                    HStack {
                        TextField("例如：论坛前缀、发布组名…", text: $newWord)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        Button("添加") {
                            let w = newWord.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !w.isEmpty else { return }
                            if !filterWords.contains(where: { $0.lowercased() == w.lowercased() }) {
                                filterWords.append(w)
                                Task {
                                    saving = true
                                    await onSave(filterWords)
                                    saving = false
                                }
                            }
                            newWord = ""
                        }
                        .disabled(newWord.trimmingCharacters(in: .whitespaces).isEmpty || saving)
                    }
                }

                Section("服务器自定义过滤词 (\(filterWords.count))") {
                    if filterWords.isEmpty {
                        Text("暂无自定义过滤词")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(filterWords, id: \.self) { word in
                            Text(word)
                        }
                        .onDelete { offsets in
                            filterWords.remove(atOffsets: offsets)
                            Task {
                                saving = true
                                await onSave(filterWords)
                                saving = false
                            }
                        }
                    }
                }

                Section("内置通用降噪停用词（自动生效）") {
                    Text(Array(MediaOrganizer.universalStopwords).sorted().prefix(40).joined(separator: "、") + " 等")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("收纳过滤词设置")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
        }
    }
}

// MARK: - 5. 从下载池(Inbox)打包新待整理项目 Sheet

struct PendingInboxPackSheet: View {
    let inboxPath: String
    let inboxRootId: String?
    let onPacked: () async -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var entries: [RemoteBrowserEntry] = []
    @State private var selectedNames: Set<String> = []
    @State private var batchName = ""
    @State private var loading = false
    @State private var packing = false
    @State private var errorText = ""

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                VStack(alignment: .leading, spacing: 8) {
                    TextField("新项目名称（例如：Pending-新批次）", text: $batchName)
                        .textFieldStyle(.roundedBorder)
                    HStack {
                        Text("源目录：\(inboxPath.isEmpty ? "未配置" : inboxPath)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Spacer()
                        Button(selectedNames.count == entries.count && !entries.isEmpty ? "取消全选" : "全选") {
                            if selectedNames.count == entries.count {
                                selectedNames.removeAll()
                            } else {
                                selectedNames = Set(entries.map(\.name))
                            }
                        }
                        .font(.caption.weight(.semibold))
                    }
                }
                .padding(14)
                .background(Color(.secondarySystemBackground))

                if !errorText.isEmpty {
                    Text(errorText)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .padding(10)
                }

                if loading && entries.isEmpty {
                    ProgressView("正在读取下载池…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if entries.isEmpty {
                    ContentUnavailableView("下载池暂无文件", systemImage: "tray", description: Text("当前云下载目录下没有可打包的文件"))
                } else {
                    List(entries) { entry in
                        let checked = selectedNames.contains(entry.name)
                        Button {
                            if checked { selectedNames.remove(entry.name) }
                            else { selectedNames.insert(entry.name) }
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: checked ? "checkmark.circle.fill" : "circle")
                                    .foregroundStyle(checked ? Color.accentColor : Color.secondary)
                                Image(systemName: entry.symbol)
                                    .foregroundStyle(.secondary)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(entry.name).font(.subheadline).lineLimit(1)
                                    if !entry.isDirectory {
                                        Text(pmlFormatBytes(UInt64(max(0, entry.sizeBytes))))
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                            }
                        }
                        .buttonStyle(.plain)
                    }
                    .listStyle(.plain)
                }
            }
            .navigationTitle("从下载池打包新项目")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(packing ? "打包中…" : "打包(\(selectedNames.count))") {
                        Task { await executePack() }
                    }
                    .fontWeight(.bold)
                    .disabled(packing || selectedNames.isEmpty || batchName.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .task { await loadInbox() }
        }
    }

    private func loadInbox() async {
        guard let client = model.apiClient, let rootId = inboxRootId, !inboxPath.isEmpty else { return }
        loading = true
        defer { loading = false }
        do {
            var page = try await client.remoteChildren(rootId: rootId, path: inboxPath, refresh: false)
            if page.items.isEmpty {
                page = try await client.remoteChildren(rootId: rootId, path: inboxPath, refresh: true)
            }
            entries = page.items.filter {
                !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@")
            }
            if batchName.isEmpty {
                let df = DateFormatter()
                df.dateFormat = "MMdd"
                batchName = "Pending-\(df.string(from: Date()))"
            }
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
    }

    private func executePack() async {
        guard let client = model.apiClient else { return }
        packing = true
        errorText = ""
        defer { packing = false }
        do {
            _ = try await client.packInbox(
                inboxPath: inboxPath,
                batchName: batchName.trimmingCharacters(in: .whitespacesAndNewlines),
                itemNames: Array(selectedNames)
            )
            await onPacked()
            dismiss()
        } catch {
            errorText = "打包失败：\(error.localizedDescription)"
        }
    }
}
