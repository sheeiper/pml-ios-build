import SwiftUI
import UIKit

private enum PendingWorkspaceTab: String, CaseIterable {
    case triage = "待归纳"
    case assigned = "已归纳"
}

private enum PendingTriageLayout {
    case deck
    case list
}

private struct PlayableVideoTarget: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
    let fallbackURL: URL?
}

private struct FolderPreviewTarget: Identifiable {
    let id = UUID()
    let rootId: String
    let path: String
    let name: String
}

struct PendingProjectCard: Identifiable, Hashable {
    let id: String
    let name: String
    let path: String
    let projectId: String?
    var totalCount: Int
    var readyCount: Int
    var isLoose: Bool
}

/// 待收纳整理：两级项目制工作台（大厅 + 项目内「待归纳/已归纳」双分区 + 服务端后台防风控移动与项目锁）
struct PendingCurationView: View {
    @EnvironmentObject private var model: AppModel

    // 目录与元数据
    @State private var pendingPath = ""
    @State private var vaultRoot = ""
    @State private var inboxPath = ""
    @State private var roots: [LibraryRoot] = []
    @State private var vaultAuthors: [VaultAuthorEntry] = []
    @State private var categoryTree: [VaultCategoryNode] = []
    @State private var filterWords: [String] = []
    @State private var savedAssignments: [String: CuratorItemAssignment?] = [:]
    @State private var metaLoaded = false

    // Level 1: 项目大厅
    @State private var projects: [PendingProjectCard] = []
    @State private var projectSearchText = ""
    @State private var loadingProjects = false

    // 服务端后台移动任务与项目锁
    @State private var moveJobs: [CurateMoveJobSnapshot] = []
    @State private var showJobsMonitor = false
    @State private var showFilterWordsSheet = false
    @State private var showInboxPackSheet = false

    // Level 2: 选中的项目双分区工作台
    @State private var selectedProject: PendingProjectCard? = nil
    @State private var allItems: [PendingCurationItem] = []
    @State private var loadingItems = false
    @State private var activeTab: PendingWorkspaceTab = .triage
    @State private var triageLayout: PendingTriageLayout = .deck

    // 刷卡台状态（待归纳）
    @State private var deckAuthorInput = ""
    @State private var deckCategoryPath = ""
    @State private var deckCategoryName = ""
    @State private var deckDirectCategoryMode = false
    @State private var stickyCategoryPath = ""
    @State private var stickyCategoryName = ""
    @State private var folderCoverAssetIds: [String: String] = [:]
    @State private var copiedTitleToast = false
    @State private var analyzingCurrentItem = false
    @State private var batchMatching = false

    // 最近常用分类持久化（跨卡片与高级选择弹窗共享）
    @AppStorage("pml.pending.recentCategories") private var recentCategoriesRaw = ""

    // 弹窗与交互
    @State private var advancedPickerItem: PendingCurationItem?
    @State private var showMoveVerifySheet = false
    @State private var activeVideoTarget: PlayableVideoTarget?
    @State private var activeImageContext: ImageViewerContext?
    @State private var activeFolderPreview: FolderPreviewTarget?
    @State private var busy = false
    @State private var errorText = ""
    @State private var hudToast: String?

    // MARK: - 计算属性

    private var filteredProjects: [PendingProjectCard] {
        let q = projectSearchText.trimmingCharacters(in: .whitespacesAndNewlines)
        if q.isEmpty { return projects }
        return projects.filter {
            $0.name.localizedCaseInsensitiveContains(q) || $0.path.localizedCaseInsensitiveContains(q)
        }
    }

    private var triageItems: [PendingCurationItem] {
        allItems.filter { !$0.isAssigned }
    }

    private var assignedItems: [PendingCurationItem] {
        allItems.filter { $0.isAssigned }
    }

    private var currentDeckItem: PendingCurationItem? {
        triageItems.first
    }

    private var runningJobs: [CurateMoveJobSnapshot] {
        moveJobs.filter { $0.isRunning }
    }

    private func runningJob(for projectPath: String) -> CurateMoveJobSnapshot? {
        moveJobs.first { $0.isRunning && $0.projectPath == projectPath }
    }

    private var selectedProjectRunningJob: CurateMoveJobSnapshot? {
        guard let project = selectedProject else { return nil }
        return runningJob(for: project.path)
    }

    /// 动态作者联合池（Federated Author Registry）：
    /// 实时聚合存量基础作者 + 服务端后台已移动任务中的新作者 + 服务端草稿作者 + 本地会话已确认归纳作者
    private var federatedAuthors: [VaultAuthorEntry] {
        var authorMap: [String: VaultAuthorEntry] = [:]
        let customSet = Set(filterWords.map { $0.lowercased() })

        // 1. 存量基础作者
        for auth in vaultAuthors {
            let key = auth.canonicalName.lowercased()
            authorMap[key] = auth
        }

        // 2. 从服务端已保存草稿中提取已知新作者（支持跨项目热同步）
        for (_, assignmentOpt) in savedAssignments {
            guard let assignment = assignmentOpt,
                  let author = assignment.author?.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] ")),
                  !author.isEmpty,
                  let targetPath = assignment.targetVaultPath else { continue }
            let key = author.lowercased()
            if authorMap[key] == nil {
                let catPath: String? = {
                    if let slash = targetPath.lastIndex(of: "/") {
                        return String(targetPath[..<slash])
                    }
                    return assignment.categoryPath
                }()
                let aliases = MediaOrganizer.splitAuthorAliases(author, customWords: customSet)
                authorMap[key] = VaultAuthorEntry(
                    id: "dynamic_\(author)",
                    canonicalName: author,
                    vaultPath: targetPath,
                    categoryPath: catPath,
                    aliases: aliases
                )
            }
        }

        // 3. 从当前已归纳条目中动态合并
        for item in assignedItems {
            guard let author = item.matchedAuthor?.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] ")),
                  !author.isEmpty,
                  let targetPath = item.targetVaultPath else { continue }
            let key = author.lowercased()
            if authorMap[key] == nil {
                let aliases = MediaOrganizer.splitAuthorAliases(author, customWords: customSet)
                authorMap[key] = VaultAuthorEntry(
                    id: "local_\(author)",
                    canonicalName: author,
                    vaultPath: targetPath,
                    categoryPath: item.matchedCategoryPath,
                    aliases: aliases
                )
            }
        }

        return Array(authorMap.values)
    }

    private var deckFinalDestination: String? {
        let catPath = deckCategoryPath.trimmingCharacters(in: .whitespacesAndNewlines)
        let authorClean = deckAuthorInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))

        // 1. 若填写了作者名
        if !authorClean.isEmpty {
            if !catPath.isEmpty {
                return "\(catPath)/【\(authorClean)】"
            }
            if let hit = federatedAuthors.first(where: { $0.canonicalName.lowercased() == authorClean.lowercased() }),
               let vp = hit.vaultPath, !vp.isEmpty {
                return vp
            }
            return vaultRoot.isEmpty ? nil : "\(vaultRoot)/【\(authorClean)】"
        }

        // 2. 若未填写作者名（留空），但选定了分类，直接归入该分类根目录（用户关键诉求：卡片免作者直接收纳）
        if !catPath.isEmpty {
            return catPath
        }

        return nil
    }

    /// 最近使用的多级分类路径列表
    private var recentCategoryPaths: [String] {
        recentCategoriesRaw.split(separator: "|").map(String.init).filter { !$0.isEmpty }
    }

    /// 所有无子节点的叶子分类（最后一级分类，优先推荐）
    private var leafCategories: [VaultCategoryNode] {
        MediaOrganizer.extractLeafCategories(categoryTree)
    }

    /// 解析最近使用的分类节点，用于胶囊展示
    private var recentCategoryNodes: [VaultCategoryNode] {
        let allFlat = MediaOrganizer.flattenCategories(categoryTree)
        var result: [VaultCategoryNode] = []
        for path in recentCategoryPaths {
            if let hit = allFlat.first(where: { $0.path == path }) {
                result.append(hit)
            } else {
                let clean = MediaOrganizer.cleanCategoryName(path)
                result.append(VaultCategoryNode(
                    id: path,
                    rawName: path,
                    cleanName: clean,
                    fullDisplayName: clean,
                    path: path,
                    parentId: nil,
                    children: []
                ))
            }
        }
        return Array(result.prefix(6))
    }

    private func recordRecentCategory(_ path: String) {
        let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        var list = recentCategoryPaths.filter { $0 != trimmed }
        list.insert(trimmed, at: 0)
        if list.count > 10 {
            list = Array(list.prefix(10))
        }
        recentCategoriesRaw = list.joined(separator: "|")
    }

    private func applyCategorySelection(_ cat: VaultCategoryNode) {
        deckCategoryPath = cat.path
        deckCategoryName = cat.fullDisplayName
        stickyCategoryPath = cat.path
        stickyCategoryName = cat.fullDisplayName
        recordRecentCategory(cat.path)
    }

    // MARK: - 主视图

    var body: some View {
        ZStack {
            if selectedProject == nil {
                projectsHallView
            } else {
                projectWorkspaceView
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
                    .padding(.top, 14)
                    Spacer()
                }
                .transition(.move(edge: .top).combined(with: .opacity))
                .animation(.spring(response: 0.32, dampingFraction: 0.8), value: hudToast)
                .allowsHitTesting(false)
            }
        }
        .navigationTitle(selectedProject?.name ?? "待整理项目")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let selected = selectedProject {
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        selectedProject = nil
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "chevron.left")
                            Text("项目大厅")
                        }
                    }
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    if selectedProjectRunningJob == nil && activeTab == .triage {
                        Button {
                            triageLayout = (triageLayout == .deck ? .list : .deck)
                        } label: {
                            Image(systemName: triageLayout == .deck ? "list.bullet" : "rectangle.stack")
                        }
                        .accessibilityLabel("切换刷卡或清单视图")
                    }
                    Button {
                        Task { await loadProjectItems(selected, forceRefresh: true) }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .disabled(loadingItems || busy || selectedProjectRunningJob != nil)
                }
            } else {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button {
                        showInboxPackSheet = true
                    } label: {
                        Image(systemName: "plus.rectangle.on.folder")
                    }
                    .accessibilityLabel("打包新项目")

                    Menu {
                        Button {
                            showJobsMonitor = true
                        } label: {
                            Label("后台移动任务 (\(runningJobs.count))", systemImage: "tray.and.arrow.up")
                        }
                        Button {
                            showFilterWordsSheet = true
                        } label: {
                            Label("过滤词设置 (\(filterWords.count))", systemImage: "line.3.horizontal.decrease.circle")
                        }
                        Button {
                            Task { await reloadAll(forceRefresh: true) }
                        } label: {
                            Label("强制刷新目录", systemImage: "arrow.clockwise")
                        }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                }
            }
        }
        .task {
            await reloadAll(forceRefresh: false)
        }
        .task {
            await pollMoveJobsLoop()
        }
        .sheet(isPresented: $showJobsMonitor) {
            PendingJobsMonitorSheet(
                jobs: moveJobs,
                onRefresh: { await refreshMoveJobs() },
                onCancelJob: { jobId in await cancelMoveJob(jobId) }
            )
        }
        .sheet(isPresented: $showFilterWordsSheet) {
            PendingFilterWordsSheet(filterWords: $filterWords) { updated in
                await saveFilterWordsToServer(updated)
            }
        }
        .sheet(isPresented: $showInboxPackSheet) {
            PendingInboxPackSheet(
                inboxPath: inboxPath,
                inboxRootId: root(for: inboxPath)?.id
            ) {
                await reloadAll(forceRefresh: true)
                showHUD("已打包新待整理项目")
            }
            .environmentObject(model)
        }
        .sheet(item: $advancedPickerItem) { item in
            PendingAdvancedPickerSheet(
                item: item,
                vaultRoot: vaultRoot,
                categoryTree: categoryTree,
                authors: vaultAuthors
            ) { result in
                applyAdvancedPickerResult(result, for: item)
            }
        }
        .sheet(isPresented: $showMoveVerifySheet) {
            moveVerifySheet
        }
        .sheet(item: $activeFolderPreview) { target in
            PendingFolderPreviewSheet(
                rootId: target.rootId,
                folderPath: target.path,
                folderName: target.name
            )
            .environmentObject(model)
        }
        .fullScreenCover(item: $activeVideoTarget) { target in
            GestureVideoPlayerSheet(
                url: target.url,
                title: target.title,
                fallbackURL: target.fallbackURL,
                sourceLabel: "待整理试看"
            )
            .ignoresSafeArea()
        }
        .fullScreenCover(item: $activeImageContext) { ctx in
            ImageViewer(context: ctx)
                .environmentObject(model)
        }
    }

    // MARK: - 第一级：待整理项目大厅

    private var projectsHallView: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                // 顶部后台移动任务状态条（有运行中任务时自动浮现）
                if !runningJobs.isEmpty {
                    Button {
                        showJobsMonitor = true
                    } label: {
                        HStack(spacing: 10) {
                            ProgressView()
                                .controlSize(.small)
                                .tint(.white)
                            VStack(alignment: .leading, spacing: 2) {
                                if let first = runningJobs.first {
                                    Text("后台正在移动【\(first.projectName)】(\(first.processed)/\(first.total))")
                                        .font(.caption.weight(.bold))
                                        .foregroundStyle(.white)
                                    Text("服务端串行防风控执行中 · 点击查看详情或停止")
                                        .font(.caption2)
                                        .foregroundStyle(.white.opacity(0.85))
                                }
                            }
                            Spacer()
                            Image(systemName: "chevron.right")
                                .font(.caption.weight(.bold))
                                .foregroundStyle(.white.opacity(0.85))
                        }
                        .padding(12)
                        .background(Color.accentColor.gradient, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 16)
                    .padding(.top, 8)
                }

                // 搜索栏
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                    TextField("搜索待整理项目…", text: $projectSearchText)
                    if !projectSearchText.isEmpty {
                        Button { projectSearchText = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .padding(10)
                .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
                .padding(.horizontal, 16)
                .padding(.top, runningJobs.isEmpty ? 8 : 0)

                // 服务器目录与汇总卡片
                HStack(spacing: 14) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("待整理项目大厅")
                            .font(.headline.weight(.bold))
                        Text("目录：\(pendingPath.isEmpty ? "未配置" : pendingPath)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Text("\(projects.count) 个待整理项目 · 任意子文件夹自动识别")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button {
                        showJobsMonitor = true
                    } label: {
                        VStack(spacing: 2) {
                            Image(systemName: runningJobs.isEmpty ? "tray.2.fill" : "bolt.horizontal.circle.fill")
                                .font(.title2)
                                .foregroundStyle(Color.accentColor)
                            if !moveJobs.isEmpty {
                                Text("任务(\(moveJobs.count))")
                                    .font(.system(size: 10, weight: .semibold))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                }
                .padding(14)
                .pmlGlassCard(cornerRadius: 18)
                .padding(.horizontal, 16)

                if !errorText.isEmpty {
                    Text(errorText)
                        .font(.footnote)
                        .foregroundStyle(.red)
                        .padding(.horizontal, 16)
                }

                if loadingProjects && projects.isEmpty {
                    VStack(spacing: 12) {
                        ProgressView()
                        Text("正在读取待整理项目…")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 44)
                } else if filteredProjects.isEmpty {
                    VStack(spacing: 14) {
                        Image(systemName: "tray")
                            .font(.system(size: 44))
                            .foregroundStyle(.secondary)
                        Text(projectSearchText.isEmpty ? "当前目录下暂无待整理项目" : "未找到匹配项目")
                            .font(.headline)
                        Text("可点击右上角从云下载打包新项目")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 60)
                } else {
                    LazyVStack(spacing: 10) {
                        ForEach(filteredProjects) { project in
                            projectCardRow(project)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.bottom, 24)
                }
            }
        }
        .refreshable {
            await reloadAll(forceRefresh: true)
        }
    }

    private func projectCardRow(_ project: PendingProjectCard) -> some View {
        let activeJob = runningJob(for: project.path)
        return Button {
            if activeJob != nil {
                selectedProject = project
            } else {
                enterProject(project)
            }
        } label: {
            HStack(spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(activeJob != nil ? Color.orange.opacity(0.16) : Color.accentColor.opacity(0.12))
                    Image(systemName: activeJob != nil ? "lock.fill" : (project.isLoose ? "tray.fill" : "folder.fill"))
                        .font(.system(size: 20))
                        .foregroundStyle(activeJob != nil ? Color.orange : Color.accentColor)
                }
                .frame(width: 46, height: 46)

                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Text(project.name)
                            .font(.subheadline.weight(.bold))
                            .lineLimit(1)
                        if project.isLoose {
                            Text("散装")
                                .font(.caption2.weight(.semibold))
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.orange.opacity(0.15), in: Capsule())
                                .foregroundStyle(.orange)
                        }
                    }
                    if let job = activeJob {
                        Text("🔒 后台移动中 \(job.processed)/\(job.total) · 锁定不可编辑")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.orange)
                    } else {
                        Text(project.path)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }

                Spacer()

                if activeJob == nil && !project.isLoose && project.totalCount == 0 {
                    Button {
                        Task { await deleteEmptyProject(project) }
                    } label: {
                        Image(systemName: "trash")
                            .font(.caption)
                            .foregroundStyle(.red)
                            .padding(6)
                            .background(Color.red.opacity(0.1), in: Circle())
                    }
                    .buttonStyle(.plain)
                }

                HStack(spacing: 4) {
                    if project.totalCount > 0 {
                        Text("\(project.totalCount)")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.bold))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(13)
            .pmlGlassCard(cornerRadius: 16)
        }
        .buttonStyle(.plain)
    }

    // MARK: - 第二级：项目内「待归纳 / 已归纳」双分区工作台

    private var projectWorkspaceView: some View {
        Group {
            if let job = selectedProjectRunningJob {
                projectLockedMovingView(job)
            } else if loadingItems && allItems.isEmpty {
                VStack(spacing: 14) {
                    ProgressView()
                    Text("正在扫描一级子项并匹配作者与分类…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if allItems.isEmpty {
                projectEmptyCelebrationView
            } else {
                VStack(spacing: 0) {
                    // 顶部双分区切换器与全项目「智能预匹配」主按钮
                    HStack(spacing: 8) {
                        Picker("分区", selection: $activeTab) {
                            Text("待归纳 (\(triageItems.count))").tag(PendingWorkspaceTab.triage)
                            Text("已归纳 (\(assignedItems.count))").tag(PendingWorkspaceTab.assigned)
                        }
                        .pickerStyle(.segmented)

                        if activeTab == .triage && !triageItems.isEmpty {
                            Button {
                                Task { await runProjectBatchAutoMatch() }
                            } label: {
                                HStack(spacing: 4) {
                                    if batchMatching {
                                        ProgressView()
                                            .controlSize(.small)
                                            .tint(.purple)
                                    } else {
                                        Image(systemName: "wand.and.stars")
                                            .font(.caption.weight(.bold))
                                    }
                                    Text(batchMatching ? "匹配中" : "智能预匹配")
                                        .font(.caption2.weight(.bold))
                                }
                                .padding(.horizontal, 10)
                                .padding(.vertical, 7)
                                .background(Color.purple.opacity(0.14), in: Capsule())
                                .foregroundStyle(.purple)
                            }
                            .buttonStyle(.plain)
                            .disabled(batchMatching || loadingItems)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)

                    if !errorText.isEmpty {
                        Text(errorText)
                            .font(.caption)
                            .foregroundStyle(.red)
                            .padding(.horizontal, 16)
                            .padding(.bottom, 6)
                    }

                    if activeTab == .triage {
                        triageZoneView
                    } else {
                        assignedZoneView
                    }
                }
            }
        }
    }

    // MARK: - 项目锁定态（后台移动中）

    private func projectLockedMovingView(_ job: CurateMoveJobSnapshot) -> some View {
        ScrollView {
            VStack(spacing: 18) {
                Spacer().frame(height: 36)
                ZStack {
                    Circle()
                        .fill(Color.orange.opacity(0.15))
                        .frame(width: 88, height: 88)
                    Image(systemName: "lock.shield.fill")
                        .font(.system(size: 42))
                        .foregroundStyle(.orange)
                }

                Text("项目已锁定 · 后台移动中")
                    .font(.title3.weight(.bold))

                Text("服务器正在单线程串行移动【\(job.projectName)】的已归纳项（每项间隔 1.3~2.0s 防 115/CD2 风控），移动完成前暂不可继续编辑该项目。")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 28)

                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("执行进度")
                            .font(.subheadline.weight(.semibold))
                        Spacer()
                        Text("\(job.processed) / \(job.total)")
                            .font(.subheadline.weight(.bold))
                            .monospacedDigit()
                    }
                    ProgressView(value: Double(job.processed), total: Double(max(1, job.total)))
                        .tint(.accentColor)

                    HStack(spacing: 14) {
                        Label("成功 \(job.successCount)", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        if job.skippedCount > 0 {
                            Label("跳过 \(job.skippedCount)", systemImage: "arrow.uturn.right.circle.fill")
                                .foregroundStyle(.orange)
                        }
                        if job.failedCount > 0 {
                            Label("失败 \(job.failedCount)", systemImage: "xmark.circle.fill")
                                .foregroundStyle(.red)
                        }
                    }
                    .font(.caption)

                    if let current = job.currentItem {
                        Text("正在移动：\(current)")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }
                .padding(16)
                .pmlGlassCard(cornerRadius: 18)
                .padding(.horizontal, 20)

                HStack(spacing: 12) {
                    Button {
                        selectedProject = nil
                    } label: {
                        Label("先去整理其他项目", systemImage: "arrow.left")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)

                    Button(role: .destructive) {
                        Task { await cancelMoveJob(job.id) }
                    } label: {
                        Label("紧急停止", systemImage: "stop.circle.fill")
                    }
                    .buttonStyle(.bordered)
                }
                .padding(.horizontal, 20)
            }
        }
    }

    // MARK: - 分区 1：待归纳（沉浸刷卡台 + 清单模式）

    private var triageZoneView: some View {
        Group {
            if triageItems.isEmpty {
                VStack(spacing: 16) {
                    Spacer()
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 52))
                        .foregroundStyle(.green)
                    Text("所有待归纳项已分类完毕！")
                        .font(.headline)
                    Text("请切换到「已归纳 (\(assignedItems.count))」核验并触发后台移动。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Button {
                        activeTab = .assigned
                    } label: {
                        Label("前往已归纳清单 (\(assignedItems.count))", systemImage: "paperplane.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    Spacer()
                }
                .padding(24)
            } else if triageLayout == .deck {
                triageDeckView
            } else {
                triageListView
            }
        }
    }

    private var triageDeckView: some View {
        ScrollView {
            VStack(spacing: 12) {
                if let item = currentDeckItem {
                    // 顶部进度与手动重新匹配栏
                    HStack(alignment: .center) {
                        HStack(spacing: 6) {
                            Image(systemName: "tray.full.fill")
                                .font(.caption)
                                .foregroundStyle(Color.accentColor)
                            Text("待归纳 \(triageItems.count) 项")
                                .font(.caption.weight(.bold))
                            if allItems.count > 0 {
                                Text("· 共 \(allItems.count) 项")
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                        }

                        Spacer()

                        Button {
                            reanalyzeCurrentDeckItem()
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.triangle.2.circlepath")
                                    .rotationEffect(analyzingCurrentItem ? .degrees(360) : .zero)
                                    .animation(analyzingCurrentItem ? .linear(duration: 0.8).repeatForever(autoreverses: false) : .default, value: analyzingCurrentItem)
                                Text(analyzingCurrentItem ? "识别中…" : "重新匹配")
                                    .font(.caption2.weight(.semibold))
                            }
                            .padding(.horizontal, 9)
                            .padding(.vertical, 5)
                            .background(Color(.secondarySystemBackground), in: Capsule())
                        }
                        .buttonStyle(.plain)
                        .disabled(analyzingCurrentItem)
                    }
                    .padding(.horizontal, 16)
                    .padding(.top, 4)

                    // 主交互卡片（已移除手势，按键驱动，标题支持长按选词部分复制）
                    deckFocusCard(for: item)
                        .padding(.horizontal, 16)
                        .task(id: item.entry.path) {
                            if !item.isAnalyzed {
                                await analyzeItemIfNeeded(item)
                            }
                            syncDeckSlots(for: item)
                            await loadCoverIfNeeded(for: item)
                        }
                }
            }
            .padding(.bottom, 24)
        }
    }

    private func deckFocusCard(for item: PendingCurationItem) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            // 封面大窗（文件夹穿透首张媒体封面 + 点击打开受控浏览器）
            Button {
                openItemMediaOrFolder(item)
            } label: {
                ZStack {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(Color(.secondarySystemFill))

                    let thumbId = item.entry.assetId ?? item.entry.coverAssetId ?? folderCoverAssetIds[item.entry.path]
                    if let thumbId {
                        AssetThumbnailView(assetId: thumbId, contentMode: .fill) {
                            fallbackMediaBanner(for: item)
                        }
                    } else {
                        fallbackMediaBanner(for: item)
                    }

                    // 居中提示胶囊
                    HStack(spacing: 6) {
                        Image(systemName: item.entry.isDirectory ? "folder.fill" : (item.entry.isVideo ? "play.fill" : "photo.fill"))
                            .font(.caption.weight(.bold))
                        Text(item.entry.isDirectory ? "点击浏览文件夹内视频/图片" : (item.entry.isVideo ? "点击全屏试看" : "点击查看大图"))
                            .font(.caption.weight(.semibold))
                    }
                    .foregroundStyle(.white)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(.ultraThinMaterial, in: Capsule())

                    // 右上角类型与体积标签
                    VStack {
                        HStack {
                            Spacer()
                            Text(item.entry.isDirectory ? "文件夹" : pmlFormatBytes(UInt64(max(0, item.entry.sizeBytes))))
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(.black.opacity(0.55), in: Capsule())
                                .padding(8)
                        }
                        Spacer()
                    }
                }
                .frame(height: 175)
                .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
            .buttonStyle(.plain)

            // 文件/文件夹名称与部分复制选词、智能切词词块
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.entry.name)
                            .font(.subheadline.weight(.bold))
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                            .lineLimit(nil)
                    }
                    Spacer(minLength: 8)
                    Button {
                        UIPasteboard.general.string = item.entry.name
                        copiedTitleToast = true
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                        Task {
                            try? await Task.sleep(nanoseconds: 1_800_000_000)
                            copiedTitleToast = false
                        }
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: copiedTitleToast ? "checkmark" : "doc.on.doc")
                                .font(.caption2)
                            Text(copiedTitleToast ? "已复制" : "复制全名")
                                .font(.caption2.weight(.medium))
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(copiedTitleToast ? Color.green.opacity(0.15) : Color(.tertiarySystemFill), in: Capsule())
                        .foregroundStyle(copiedTitleToast ? Color.green : Color.primary)
                    }
                    .buttonStyle(.plain)
                }

                // 智能分词词块（Word Chips）：点选直接填入作者输入框
                if !item.candidateAuthorChips.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("智能提取词块（点击直接填入作者）：")
                            .font(.caption2)
                            .foregroundStyle(.secondary)

                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 6) {
                                ForEach(item.candidateAuthorChips, id: \.self) { chip in
                                    let selected = !deckDirectCategoryMode && deckAuthorInput.lowercased() == chip.lowercased()
                                    Button {
                                        deckDirectCategoryMode = false
                                        deckAuthorInput = chip
                                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
                                    } label: {
                                        HStack(spacing: 3) {
                                            if selected {
                                                Image(systemName: "checkmark")
                                                    .font(.system(size: 9, weight: .bold))
                                            }
                                            Text(chip)
                                                .font(.caption2.weight(selected ? .bold : .medium))
                                        }
                                        .padding(.horizontal, 9)
                                        .padding(.vertical, 5)
                                        .background(selected ? Color.purple : Color.purple.opacity(0.14), in: Capsule())
                                        .foregroundStyle(selected ? .white : .purple)
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    }
                    .padding(.top, 2)
                }
            }
            .padding(10)
            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))

            // 紫色作者槽位：输入框 + 一键粘贴 + 免作者自动直接收纳
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Label("作者槽位", systemImage: "person.fill")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.purple)
                    let cleanAuth = deckAuthorInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))
                    if !cleanAuth.isEmpty {
                        Text("已选：\(cleanAuth)")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.purple)
                            .lineLimit(1)
                    } else {
                        Text("未填（确认归纳时将直接放在分类根目录下）")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if !deckAuthorInput.isEmpty {
                        Button {
                            deckAuthorInput = ""
                            UIImpactFeedbackGenerator(style: .light).impactOccurred()
                        } label: {
                            Text("清空免作者")
                                .font(.caption2.weight(.medium))
                                .padding(.horizontal, 8)
                                .padding(.vertical, 4)
                                .background(Color.purple.opacity(0.12), in: Capsule())
                                .foregroundStyle(.purple)
                        }
                        .buttonStyle(.plain)
                    }
                }

                HStack(spacing: 6) {
                    Image(systemName: "pencil")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    TextField("输入或点选上方词块设定作者（选填，不填直接收纳至分类）…", text: $deckAuthorInput)
                        .font(.caption)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    if !deckAuthorInput.isEmpty {
                        Button { deckAuthorInput = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                    Button {
                        if let clip = UIPasteboard.general.string?.trimmingCharacters(in: .whitespacesAndNewlines), !clip.isEmpty {
                            deckAuthorInput = clip
                            UIImpactFeedbackGenerator(style: .light).impactOccurred()
                        }
                    } label: {
                        Image(systemName: "doc.on.clipboard")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.purple)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("粘贴剪贴板文字为作者")
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10))
            }

            // 绿色分类槽位：最近分类推荐 + 库内叶子分类 + 优先最后一级
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Label("分类槽位（优先最后一级叶子）", systemImage: "folder.fill")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.green)
                    if !deckCategoryName.isEmpty {
                        let displayLeaf = deckCategoryName.components(separatedBy: " / ").last ?? deckCategoryName
                        Text("已选：\(displayLeaf)")
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(.green)
                            .lineLimit(1)
                    }
                    Spacer()
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 7) {
                        // 1. 若命中了分类，首位专属高亮胶囊
                        if !deckCategoryPath.isEmpty {
                            let leafName = deckCategoryName.components(separatedBy: " / ").last ?? MediaOrganizer.cleanCategoryName(deckCategoryPath)
                            Button {
                                // 保持已选
                            } label: {
                                HStack(spacing: 4) {
                                    Image(systemName: "checkmark.seal.fill")
                                        .font(.caption2)
                                    Text(leafName)
                                        .font(.caption.weight(.bold))
                                }
                                .padding(.horizontal, 12)
                                .padding(.vertical, 6)
                                .background(Color.green, in: Capsule())
                                .foregroundStyle(.white)
                            }
                            .buttonStyle(.plain)
                        }

                        // 2. 最近使用分类胶囊（跨高级选择共享持久化）
                        ForEach(recentCategoryNodes) { cat in
                            let isCurrent = deckCategoryPath == cat.path
                            if !isCurrent {
                                Button {
                                    applyCategorySelection(cat)
                                } label: {
                                    HStack(spacing: 3) {
                                        Image(systemName: "clock.arrow.circlepath")
                                            .font(.system(size: 9))
                                        Text(cat.cleanName)
                                            .font(.caption.weight(.medium))
                                    }
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(Color.green.opacity(0.14), in: Capsule())
                                    .foregroundStyle(Color.green)
                                }
                                .buttonStyle(.plain)
                            }
                        }

                        // 3. 库中所有叶子分类节点（保证落在最后一级分类）
                        ForEach(leafCategories.prefix(12)) { cat in
                            let isCurrent = deckCategoryPath == cat.path
                            let inRecent = recentCategoryNodes.contains(where: { $0.path == cat.path })
                            if !isCurrent && !inRecent {
                                Button {
                                    applyCategorySelection(cat)
                                } label: {
                                    Text(cat.cleanName)
                                        .font(.caption.weight(.medium))
                                        .padding(.horizontal, 10)
                                        .padding(.vertical, 6)
                                        .background(Color(.secondarySystemBackground), in: Capsule())
                                        .foregroundStyle(.secondary)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            }

            // 实时预定归宿预览条
            VStack(alignment: .leading, spacing: 3) {
                Text("预定归宿")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.secondary)
                if let dest = deckFinalDestination {
                    let cleanAuth = deckAuthorInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))
                    HStack(spacing: 6) {
                        Text("→ \(dest)")
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(Color.accentColor)
                            .lineLimit(2)
                        if cleanAuth.isEmpty {
                            Text("(免作者·直接归入分类)")
                                .font(.system(size: 10, weight: .bold))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(Color.orange.opacity(0.15), in: Capsule())
                                .foregroundStyle(.orange)
                        }
                    }
                } else {
                    Text("请点选下方分类（可免填作者直接归入，或输入作者生成专属目录）")
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(Color.secondary)
                        .lineLimit(2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))

            // 底部大拇指高效操作栏（跳过 / 高级… / 确认归纳移入）
            HStack(spacing: 10) {
                Button {
                    skipCurrentDeckItem()
                } label: {
                    VStack(spacing: 3) {
                        Image(systemName: "forward.fill")
                            .font(.body.weight(.bold))
                        Text("跳过")
                            .font(.caption2.weight(.medium))
                    }
                    .frame(minWidth: 64)
                    .padding(.vertical, 10)
                }
                .buttonStyle(.bordered)
                .tint(.secondary)
                .disabled(triageItems.count <= 1)

                Button {
                    advancedPickerItem = item
                } label: {
                    VStack(spacing: 3) {
                        Image(systemName: "slider.horizontal.3")
                            .font(.body.weight(.bold))
                        Text("高级…")
                            .font(.caption2.weight(.medium))
                    }
                    .frame(minWidth: 64)
                    .padding(.vertical, 10)
                }
                .buttonStyle(.bordered)

                Button {
                    confirmCurrentDeckItemToAssigned()
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.title3)
                        Text("确认归纳移入")
                            .font(.subheadline.weight(.bold))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                }
                .buttonStyle(.borderedProminent)
                .tint(.accentColor)
                .disabled(deckFinalDestination == nil)
            }
        }
        .padding(14)
        .pmlGlassCard(cornerRadius: 20)
    }

    private func fallbackMediaBanner(for item: PendingCurationItem) -> some View {
        ZStack {
            LinearGradient(
                colors: [Color.accentColor.opacity(0.25), Color.purple.opacity(0.2)],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            Image(systemName: item.entry.isDirectory ? "folder.fill" : (item.entry.isVideo ? "video.fill" : "photo.fill"))
                .font(.system(size: 42))
                .foregroundStyle(.white.opacity(0.45))
        }
    }

    private var triageListView: some View {
        List(triageItems) { item in
            HStack(spacing: 12) {
                Button {
                    openItemMediaOrFolder(item)
                } label: {
                    Image(systemName: item.entry.symbol)
                        .font(.title3)
                        .foregroundStyle(Color.accentColor)
                        .frame(width: 36, height: 36)
                        .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
                }
                .buttonStyle(.plain)

                VStack(alignment: .leading, spacing: 4) {
                    Text(item.entry.name)
                        .font(.subheadline.weight(.medium))
                        .lineLimit(2)
                    HStack(spacing: 6) {
                        if let catName = item.matchedCategoryName {
                            Text("预记分类：\(catName)")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.green)
                        }
                        if !item.candidateAuthorChips.isEmpty {
                            Text("候选：\(item.candidateAuthorChips.prefix(2).joined(separator: " / "))")
                                .font(.caption2)
                                .foregroundStyle(.purple)
                                .lineLimit(1)
                        }
                    }
                }

                Spacer()

                Button("指定") {
                    advancedPickerItem = item
                }
                .font(.caption.weight(.semibold))
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            }
            .padding(.vertical, 4)
        }
        .listStyle(.plain)
    }

    // MARK: - 分区 2：已归纳（核验、改动、退回待归纳、一键后台移动）

    private var assignedZoneView: some View {
        ZStack(alignment: .bottom) {
            if assignedItems.isEmpty {
                VStack(spacing: 14) {
                    Spacer()
                    Image(systemName: "tray.and.arrow.down")
                        .font(.system(size: 46))
                        .foregroundStyle(.secondary)
                    Text("已归纳清单为空")
                        .font(.headline)
                    Text("命中已有作者的项会自动出现在这里，也可在「待归纳」选定分类与作者后加入。")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 32)
                    Button("前往待归纳 (\(triageItems.count))") {
                        activeTab = .triage
                    }
                    .buttonStyle(.borderedProminent)
                    Spacer()
                }
            } else {
                List {
                    ForEach(assignedItems) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(alignment: .top, spacing: 8) {
                                Image(systemName: item.entry.symbol)
                                    .foregroundStyle(Color.accentColor)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(item.entry.name)
                                        .font(.subheadline.weight(.semibold))
                                        .lineLimit(2)
                                    HStack(spacing: 6) {
                                        Text(item.assignmentSource.badgeText)
                                            .font(.caption2.weight(.bold))
                                            .padding(.horizontal, 6)
                                            .padding(.vertical, 2)
                                            .background(Color.accentColor.opacity(0.12), in: Capsule())
                                            .foregroundStyle(Color.accentColor)
                                        if let author = item.matchedAuthor, !author.isEmpty {
                                            Text("【\(author)】")
                                                .font(.caption2.weight(.bold))
                                                .foregroundStyle(.purple)
                                        }
                                    }
                                    if let dest = item.targetVaultPath {
                                        Text("→ \(dest)")
                                            .font(.system(.caption2, design: .monospaced))
                                            .foregroundStyle(.secondary)
                                            .lineLimit(2)
                                    }
                                }
                                Spacer()
                            }

                            HStack(spacing: 10) {
                                Button {
                                    advancedPickerItem = item
                                } label: {
                                    Label("改分类/作者", systemImage: "pencil")
                                        .font(.caption2.weight(.semibold))
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)

                                Button(role: .destructive) {
                                    revertItemToTriage(item)
                                } label: {
                                    Label("退回待归纳", systemImage: "arrow.uturn.backward")
                                        .font(.caption2.weight(.semibold))
                                }
                                .buttonStyle(.bordered)
                                .controlSize(.small)

                                Spacer()
                            }
                        }
                        .padding(.vertical, 4)
                        .swipeActions(edge: .trailing) {
                            Button("退回", role: .destructive) {
                                revertItemToTriage(item)
                            }
                        }
                    }
                    // 底部留白避免被悬浮按钮遮挡
                    Color.clear.frame(height: 74).listRowSeparator(.hidden)
                }
                .listStyle(.plain)

                // 底部悬浮：一键后台移动主按钮
                VStack(spacing: 0) {
                    Button {
                        showMoveVerifySheet = true
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "paperplane.fill")
                            Text("一键后台移动（\(assignedItems.count) 项）")
                                .fontWeight(.bold)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 14)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(.green)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 12)
                    .background(.ultraThinMaterial)
                }
            }
        }
    }

    // MARK: - 批量后台移动核验确认 Sheet

    private var moveVerifySheet: some View {
        NavigationStack {
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "shield.checkered")
                        .font(.title2)
                        .foregroundStyle(.green)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("服务端后台串行防风控移动")
                            .font(.subheadline.weight(.bold))
                        Text("共 \(assignedItems.count) 项，每项随机延迟 1.3~2.0s，同名自动跳过不覆盖。移动期间本项目自动锁定，可锁屏或切去整理其他项目。")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.green.opacity(0.1))

                List(assignedItems) { item in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.entry.name)
                            .font(.caption.weight(.semibold))
                            .lineLimit(2)
                        Text("→ \(item.targetVaultPath ?? "")")
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(Color.accentColor)
                            .lineLimit(2)
                    }
                }
                .listStyle(.plain)
            }
            .navigationTitle("核验并启动后台移动")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") { showMoveVerifySheet = false }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button(busy ? "提交中…" : "开始后台移动") {
                        Task { await triggerServerBackgroundMove() }
                    }
                    .fontWeight(.bold)
                    .disabled(busy || assignedItems.isEmpty)
                }
            }
        }
    }

    // MARK: - 空项目庆祝视图

    private var projectEmptyCelebrationView: some View {
        VStack(spacing: 18) {
            Spacer()
            ZStack {
                Circle()
                    .fill(Color.green.opacity(0.14))
                    .frame(width: 86, height: 86)
                Image(systemName: "checkmark.seal.fill")
                    .font(.system(size: 46))
                    .foregroundStyle(.green)
            }
            Text("【\(selectedProject?.name ?? "该项目")】已全部清空！")
                .font(.title3.weight(.bold))
            Text("该项目下已无待整理子项，您可以安全清理此空目录或返回大厅。")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)

            HStack(spacing: 12) {
                if let project = selectedProject, !project.isLoose {
                    Button(role: .destructive) {
                        Task {
                            await deleteEmptyProject(project)
                            selectedProject = nil
                        }
                    } label: {
                        Label("清理空项目", systemImage: "trash")
                    }
                    .buttonStyle(.bordered)
                }

                Button {
                    selectedProject = nil
                } label: {
                    Label("返回项目大厅", systemImage: "arrow.left")
                }
                .buttonStyle(.borderedProminent)
            }
            Spacer()
        }
    }

    // MARK: - 交互逻辑：刷卡、传播、退回、高级选择、持久化

    private func syncDeckSlots(for item: PendingCurationItem) {
        deckDirectCategoryMode = false
        // 1. 同步作者：已命中作者优先；未命中时若提取到候选词，自动预选第 1 个候选词
        if let matched = item.matchedAuthor, !matched.isEmpty {
            deckAuthorInput = matched
        } else if let firstChip = item.candidateAuthorChips.first, !firstChip.isEmpty {
            deckAuthorInput = firstChip
        } else {
            deckAuthorInput = ""
        }

        // 2. 同步分类：优先使用条目命中的最深层分类；若无，使用上次记忆分类
        if let cp = item.matchedCategoryPath, !cp.isEmpty {
            deckCategoryPath = cp
            deckCategoryName = item.matchedCategoryName ?? MediaOrganizer.cleanCategoryName(cp)
        } else if !stickyCategoryPath.isEmpty {
            deckCategoryPath = stickyCategoryPath
            deckCategoryName = stickyCategoryName
        } else {
            deckCategoryPath = ""
            deckCategoryName = ""
        }
    }

    private func confirmCurrentDeckItemToAssigned() {
        guard let item = currentDeckItem, let dest = deckFinalDestination else { return }
        let authorClean: String? = {
            let s = deckAuthorInput.trimmingCharacters(in: CharacterSet(charactersIn: "【】[] "))
            return s.isEmpty ? nil : s
        }()
        let catPath = deckCategoryPath.isEmpty ? nil : deckCategoryPath
        let catName = deckCategoryName.isEmpty ? nil : deckCategoryName

        if let cp = catPath {
            recordRecentCategory(cp)
        }

        assignItem(
            item,
            targetVaultPath: dest,
            author: authorClean,
            categoryPath: catPath,
            categoryName: catName,
            source: .manual
        )
    }

    private func applyAdvancedPickerResult(_ result: PendingAdvancedPickerResult, for item: PendingCurationItem) {
        if let newCat = result.newCategoryCreated {
            categoryTree.append(newCat)
        }
        if let aliasUpdate = result.updatedAliases, let client = model.apiClient {
            Task { try? await client.updateAuthorAliases(authorId: aliasUpdate.authorId, aliases: aliasUpdate.aliases) }
        }
        if let cp = result.categoryPath, !cp.isEmpty {
            stickyCategoryPath = cp
            stickyCategoryName = result.categoryName ?? MediaOrganizer.cleanCategoryName(cp)
            recordRecentCategory(cp)
        }
        assignItem(
            item,
            targetVaultPath: result.targetVaultPath,
            author: result.author,
            categoryPath: result.categoryPath,
            categoryName: result.categoryName,
            source: .manual
        )
    }

    /// 整项目手动「智能批量预匹配」：
    /// 将本项目所有未归纳项与动态作者联合池进行全量智能匹配，命中已知作者的自动直接归纳
    private func runProjectBatchAutoMatch() async {
        guard !batchMatching, let project = selectedProject else { return }
        batchMatching = true
        defer { batchMatching = false }

        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        // 确保元数据最新
        await ensureMetaLoaded()
        let activeAuthors = federatedAuthors
        let customSet = filterWords

        var autoAssignedCount = 0
        var newAssignments: [String: CuratorItemAssignment?] = [:]

        for i in allItems.indices where !allItems[i].isAssigned {
            let analyzed = MediaOrganizer.analyzePendingEntry(
                allItems[i].entry,
                authors: activeAuthors,
                categoryTree: categoryTree,
                vaultRoot: vaultRoot,
                customFilterWords: customSet,
                savedAssignments: savedAssignments
            )
            allItems[i] = analyzed
            if analyzed.isAssigned, let dest = analyzed.targetVaultPath {
                autoAssignedCount += 1
                newAssignments[analyzed.entry.path] = CuratorItemAssignment(
                    targetVaultPath: dest,
                    author: analyzed.matchedAuthor,
                    categoryPath: analyzed.matchedCategoryPath,
                    categoryName: analyzed.matchedCategoryName
                )
            }
        }

        // 同步持久化到服务端草稿
        if !newAssignments.isEmpty {
            for (k, v) in newAssignments {
                savedAssignments[k] = v
            }
            if let client = model.apiClient {
                Task { try? await client.saveCuratorAssignments(newAssignments) }
            }
        }

        // 更新项目卡片计数
        if let idx = projects.firstIndex(where: { $0.id == project.id }) {
            projects[idx].totalCount = allItems.count
            projects[idx].readyCount = assignedItems.count
            selectedProject?.readyCount = assignedItems.count
        }

        // 若当前展示的卡片还在待归纳区，同步槽位
        if let current = currentDeckItem {
            syncDeckSlots(for: current)
        }

        UINotificationFeedbackGenerator().notificationOccurred(.success)
        let remaining = triageItems.count
        if autoAssignedCount > 0 {
            showHUD("✓ 预匹配完成：已自动归纳 \(autoAssignedCount) 项，剩余 \(remaining) 项待确认")
        } else {
            showHUD("预匹配完成：未发现已知作者，已推荐分类与候选词")
        }
    }

    /// 按需即时识别单个条目（秒开后当前展示项即时匹配）
    private func analyzeItemIfNeeded(_ item: PendingCurationItem, force: Bool = false) async {
        guard !item.isAnalyzed || force else { return }
        let analyzed = MediaOrganizer.analyzePendingEntry(
            item.entry,
            authors: federatedAuthors,
            categoryTree: categoryTree,
            vaultRoot: vaultRoot,
            customFilterWords: filterWords,
            savedAssignments: savedAssignments
        )
        await MainActor.run {
            if let idx = allItems.firstIndex(where: { $0.id == item.id }) {
                allItems[idx] = analyzed
                if currentDeckItem?.id == item.id {
                    syncDeckSlots(for: analyzed)
                }
                if let pIdx = projects.firstIndex(where: { $0.id == selectedProject?.id }) {
                    projects[pIdx].readyCount = assignedItems.count
                    selectedProject?.readyCount = assignedItems.count
                }
            }
        }
    }

    /// 用户在卡片上主动点击「重新匹配」
    private func reanalyzeCurrentDeckItem() {
        guard let item = currentDeckItem else { return }
        analyzingCurrentItem = true
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        Task {
            await analyzeItemIfNeeded(item, force: true)
            analyzingCurrentItem = false
            showHUD("已重新识别分类与作者")
        }
    }

    private func assignItem(
        _ item: PendingCurationItem,
        targetVaultPath: String,
        author: String?,
        categoryPath: String?,
        categoryName: String?,
        source: PendingAssignmentSource
    ) {
        guard let idx = allItems.firstIndex(where: { $0.id == item.id }) else { return }
        allItems[idx].targetVaultPath = targetVaultPath
        allItems[idx].matchedAuthor = author
        allItems[idx].matchedCategoryPath = categoryPath
        allItems[idx].matchedCategoryName = categoryName
        allItems[idx].assignmentSource = source

        var toPersist: [String: CuratorItemAssignment?] = [
            item.entry.path: CuratorItemAssignment(
                targetVaultPath: targetVaultPath,
                author: author,
                categoryPath: categoryPath,
                categoryName: categoryName
            )
        ]

        // 同作者智能传播：自动把本批次其余包含同一作者名的未归纳项一并带入已归纳
        var propagatedCount = 0
        if let author, author.count >= 2 {
            let lowerAuth = author.lowercased()
            for i in allItems.indices where !allItems[i].isAssigned {
                if allItems[i].entry.name.lowercased().contains(lowerAuth) {
                    allItems[i].targetVaultPath = targetVaultPath
                    allItems[i].matchedAuthor = author
                    allItems[i].matchedCategoryPath = categoryPath
                    allItems[i].matchedCategoryName = categoryName
                    allItems[i].assignmentSource = .propagated
                    toPersist[allItems[i].entry.path] = CuratorItemAssignment(
                        targetVaultPath: targetVaultPath,
                        author: author,
                        categoryPath: categoryPath,
                        categoryName: categoryName
                    )
                    propagatedCount += 1
                }
            }
        }

        // 同步持久化到服务端 curator_config.manualAssignments
        for (k, v) in toPersist {
            savedAssignments[k] = v
        }
        if let client = model.apiClient {
            let payload = toPersist
            Task { try? await client.saveCuratorAssignments(payload) }
        }

        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        if propagatedCount > 0 {
            showHUD("已归入已归纳（同作者自动联动 +\(propagatedCount) 项）")
        } else {
            showHUD("✓ 已加入已归纳清单")
        }
    }

    private func revertItemToTriage(_ item: PendingCurationItem) {
        guard let idx = allItems.firstIndex(where: { $0.id == item.id }) else { return }
        allItems[idx].targetVaultPath = nil
        allItems[idx].matchedAuthor = nil
        allItems[idx].matchedAlias = nil
        allItems[idx].assignmentSource = .none
        savedAssignments.removeValue(forKey: item.entry.path)
        if let client = model.apiClient {
            let path = item.entry.path
            Task { try? await client.saveCuratorAssignments([path: nil]) }
        }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        showHUD("已退回待归纳")
    }

    private func skipCurrentDeckItem() {
        guard let first = currentDeckItem,
              let idx = allItems.firstIndex(where: { $0.id == first.id }),
              triageItems.count > 1 else { return }
        let removed = allItems.remove(at: idx)
        allItems.append(removed)
    }

    private func openItemMediaOrFolder(_ item: PendingCurationItem) {
        if item.entry.isDirectory {
            guard let rootId = root(for: item.entry.path)?.id else { return }
            activeFolderPreview = FolderPreviewTarget(rootId: rootId, path: item.entry.path, name: item.entry.name)
            return
        }
        if item.entry.isVideo {
            Task {
                if let source = await model.playbackSource(assetId: item.entry.assetId, remotePath: item.entry.path) {
                    activeVideoTarget = PlayableVideoTarget(url: source.url, title: item.entry.name, fallbackURL: source.fallbackURL)
                }
            }
            return
        }
        if item.entry.isImage {
            activeImageContext = ImageViewerContext(entries: [item.entry], startIndex: 0)
        }
    }

    /// 文件夹封面穿透：仅异步探测文件夹内第一个有缩略图的媒体（不作为任务项展开）
    private func loadCoverIfNeeded(for item: PendingCurationItem) async {
        guard item.entry.isDirectory,
              folderCoverAssetIds[item.entry.path] == nil,
              let client = model.apiClient,
              let rootObj = root(for: item.entry.path) else { return }
        if let page = try? await client.remoteChildren(rootId: rootObj.id, path: item.entry.path, refresh: false) {
            if let firstMedia = page.items.first(where: { ($0.assetId != nil || $0.coverAssetId != nil) }) {
                if let id = firstMedia.coverAssetId ?? firstMedia.assetId {
                    folderCoverAssetIds[item.entry.path] = id
                }
            }
        }
    }

    // MARK: - 服务端后台移动任务与轮询

    private func triggerServerBackgroundMove() async {
        guard let client = model.apiClient, let project = selectedProject else { return }
        let payload: [(source: String, target: String, isDirectory: Bool)] = assignedItems.compactMap { item in
            guard let target = item.finalMoveTargetPath else { return nil }
            return (source: item.entry.path, target: target, isDirectory: item.entry.isDirectory)
        }
        guard !payload.isEmpty else { return }
        busy = true
        defer { busy = false }
        do {
            let job = try await client.startCurateMoveJob(
                projectPath: project.path,
                projectName: project.name,
                items: payload,
                delayMs: 1300
            )
            moveJobs.removeAll { $0.id == job.id }
            moveJobs.insert(job, at: 0)
            showMoveVerifySheet = false
            showHUD("已启动服务器后台移动，该项目已锁定")
        } catch {
            errorText = "启动后台移动失败：\(error.localizedDescription)"
            showMoveVerifySheet = false
        }
    }

    private func cancelMoveJob(_ jobId: String) async {
        guard let client = model.apiClient else { return }
        if let updated = try? await client.cancelCurateMoveJob(jobId: jobId) {
            if let idx = moveJobs.firstIndex(where: { $0.id == updated.id }) {
                moveJobs[idx] = updated
            }
            showHUD("已发送紧急停止指令")
        }
    }

    private func refreshMoveJobs() async {
        guard let client = model.apiClient else { return }
        let hadRunning = !runningJobs.isEmpty
        let runningProjectPaths = Set(runningJobs.map(\.projectPath))
        if let latest = try? await client.listCurateMoveJobs() {
            moveJobs = latest
            // 如果之前正在移动的某个项目刚刚完成了，自动刷新该项目/大厅
            let stillRunningPaths = Set(latest.filter(\.isRunning).map(\.projectPath))
            let justFinished = runningProjectPaths.subtracting(stillRunningPaths)
            if hadRunning && !justFinished.isEmpty {
                if let selected = selectedProject, justFinished.contains(selected.path) {
                    await loadProjectItems(selected, forceRefresh: true)
                    showHUD("【\(selected.name)】后台移动任务已结束")
                } else {
                    await discoverProjects(forceRefresh: true)
                }
            }
        }
    }

    private func pollMoveJobsLoop() async {
        while !Task.isCancelled {
            let interval: UInt64 = runningJobs.isEmpty ? 8_000_000_000 : 2_000_000_000
            try? await Task.sleep(nanoseconds: interval)
            if Task.isCancelled { break }
            await refreshMoveJobs()
        }
    }

    // MARK: - 数据加载与配置持久化

    private func reloadAll(forceRefresh: Bool) async {
        guard let client = model.apiClient else { return }
        loadingProjects = true
        errorText = ""
        defer { loadingProjects = false }
        do {
            let config = try await client.directoryConfig()
            pendingPath = config.pendingPath ?? ""
            vaultRoot = config.vaultPath ?? ""
            inboxPath = config.recentPath ?? ""
            roots = try await client.libraryRoots()

            if let curator = try? await client.curatorConfig() {
                filterWords = curator.filterWords ?? []
                savedAssignments = curator.manualAssignments ?? [:]
            }

            await refreshMoveJobs()
            await discoverProjects(forceRefresh: forceRefresh)
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
    }

    private func ensureMetaLoaded() async {
        guard !metaLoaded, let client = model.apiClient else { return }
        let customSet = Set(filterWords.map { $0.lowercased() })
        if let meta = try? await client.domainMeta() {
            categoryTree = MediaOrganizer.buildCategoryTree(from: meta.categories)
            vaultAuthors = MediaOrganizer.buildAuthorEntries(from: meta.authors, customWords: customSet)
        }
        metaLoaded = true
    }

    /// 取消 Pending 命名限制：待整理根目录下的任意可见子文件夹均为待整理项目；散装直属文件自动聚合为「散装文件」
    private func discoverProjects(forceRefresh: Bool) async {
        guard let client = model.apiClient else { return }
        var cardMap: [String: PendingProjectCard] = [:]

        if !pendingPath.isEmpty, let pendingRoot = root(for: pendingPath) {
            var page = try? await client.remoteChildren(rootId: pendingRoot.id, path: pendingPath, refresh: forceRefresh)
            if !forceRefresh, (page?.items.isEmpty ?? true) {
                page = try? await client.remoteChildren(rootId: pendingRoot.id, path: pendingPath, refresh: true)
            }
            let visible = (page?.items ?? []).filter {
                !$0.name.hasPrefix(".") && !$0.name.hasPrefix("@") && $0.name != "desktop.ini" && $0.name != "Thumbs.db"
            }

            let folderProjects = visible.filter { $0.isDirectory }
            let looseFiles = visible.filter { !$0.isDirectory }

            for entry in folderProjects {
                cardMap[entry.path] = PendingProjectCard(
                    id: entry.path,
                    name: entry.name,
                    path: entry.path,
                    projectId: nil,
                    totalCount: 0,
                    readyCount: 0,
                    isLoose: false
                )
            }

            if !looseFiles.isEmpty {
                cardMap[pendingPath] = PendingProjectCard(
                    id: pendingPath,
                    name: "散装文件",
                    path: pendingPath,
                    projectId: nil,
                    totalCount: looseFiles.count,
                    readyCount: 0,
                    isLoose: true
                )
            }
        }

        // 合并数据库 pending_projects 的统计数（仅用于展示徽章，不再过滤目录名）
        if let domainProjects = try? await client.pendingProjects() {
            for dp in domainProjects {
                if let existing = cardMap[dp.sourcePath] {
                    cardMap[dp.sourcePath] = PendingProjectCard(
                        id: existing.id,
                        name: existing.name,
                        path: existing.path,
                        projectId: dp.id,
                        totalCount: existing.totalCount > 0 ? existing.totalCount : Int(dp.totalCount),
                        readyCount: Int(dp.readyCount),
                        isLoose: existing.isLoose || dp.isLoose
                    )
                }
            }
        }

        projects = cardMap.values.sorted { a, b in
            if a.isLoose != b.isLoose { return !a.isLoose }
            return a.name.localizedStandardCompare(b.name) == .orderedAscending
        }
    }

    private func enterProject(_ project: PendingProjectCard) {
        selectedProject = project
        activeTab = .triage
        Task { await loadProjectItems(project, forceRefresh: false) }
    }

    private func loadProjectItems(_ project: PendingProjectCard, forceRefresh: Bool) async {
        loadingItems = true
        errorText = ""
        allItems = []
        defer { loadingItems = false }
        guard let client = model.apiClient else { return }

        do {
            // 按需加载极速元数据（0.01s）与最新过滤词/草稿
            await ensureMetaLoaded()
            if let curator = try? await client.curatorConfig() {
                filterWords = curator.filterWords ?? []
                savedAssignments = curator.manualAssignments ?? [:]
            }

            // 仅探测项目下第一层直接子项（绝不再往下探测）
            var loadedEntries: [RemoteBrowserEntry] = []
            if let pendingRoot = root(for: project.path) {
                var page = try await client.remoteChildren(rootId: pendingRoot.id, path: project.path, refresh: forceRefresh)
                if !forceRefresh && page.items.isEmpty {
                    page = try await client.remoteChildren(rootId: pendingRoot.id, path: project.path, refresh: true)
                }
                loadedEntries = page.items.filter { entry in
                    guard !entry.name.hasPrefix("."), !entry.name.hasPrefix("@"),
                          entry.name != "desktop.ini", entry.name != "Thumbs.db" else { return false }
                    // 如果是「散装文件」虚拟项目，只取根目录下的直属文件，不取子项目文件夹
                    if project.isLoose && project.path == pendingPath && entry.isDirectory {
                        return false
                    }
                    return true
                }
                .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
            }

            // 两阶段秒开加载：只从服务端恢复已保存草稿，其余子项轻量初始化，进入待整理 0 秒延迟！
            allItems = loadedEntries.map { entry in
                if let optionalSaved = savedAssignments[entry.path],
                   let saved = optionalSaved,
                   let savedTarget = saved.targetVaultPath,
                   !savedTarget.isEmpty {
                    return PendingCurationItem(
                        entry: entry,
                        assignmentSource: .restoredDraft,
                        matchedAuthorId: nil,
                        matchedAuthor: saved.author,
                        matchedAlias: nil,
                        matchedCategoryPath: saved.categoryPath,
                        matchedCategoryName: saved.categoryName,
                        targetVaultPath: savedTarget,
                        candidateAuthorChips: [],
                        isAnalyzed: true
                    )
                } else {
                    return PendingCurationItem(entry: entry, isAnalyzed: false)
                }
            }

            if let idx = projects.firstIndex(where: { $0.id == project.id }) {
                projects[idx].totalCount = allItems.count
                projects[idx].readyCount = assignedItems.count
                selectedProject?.totalCount = allItems.count
                selectedProject?.readyCount = assignedItems.count
            }

            // 如果所有项都已经直接命中作者进入了「已归纳」，自动切到已归纳标签方便一键移动
            if triageItems.isEmpty && !assignedItems.isEmpty {
                activeTab = .assigned
            }
        } catch {
            if !pmlIsCancellation(error) {
                errorText = "加载项目子项失败：\(error.localizedDescription)"
            }
        }
    }

    private func saveFilterWordsToServer(_ words: [String]) async {
        guard let client = model.apiClient else { return }
        try? await client.saveCuratorFilterWords(words)
        metaLoaded = false
        showHUD("已保存过滤词配置")
        if let selected = selectedProject {
            await loadProjectItems(selected, forceRefresh: false)
        }
    }

    private func deleteEmptyProject(_ project: PendingProjectCard) async {
        guard let client = model.apiClient, !project.isLoose else { return }
        do {
            try await client.deleteEmptyPendingBatch(batchPath: project.path)
            projects.removeAll { $0.id == project.id }
            showHUD("已清理空项目【\(project.name)】")
        } catch {
            errorText = "清理失败：\(error.localizedDescription)"
        }
    }

    private func root(for path: String) -> LibraryRoot? {
        roots
            .filter { path == $0.remotePath || path.hasPrefix($0.remotePath + "/") }
            .sorted { $0.remotePath.count > $1.remotePath.count }
            .first
    }

    private func showHUD(_ text: String) {
        hudToast = text
        Task {
            try? await Task.sleep(nanoseconds: 1_800_000_000)
            await MainActor.run {
                if hudToast == text { hudToast = nil }
            }
        }
    }
}
