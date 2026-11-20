import SwiftUI

public struct CleanupView: View {
    @EnvironmentObject private var appModel: AppModel

    @State private var whitelist: [String] = []
    @State private var newPath: String = ""
    @State private var savingWhitelist = false
    @State private var whitelistNotice: String?

    @State private var scanning = false
    @State private var deleting = false
    @State private var cleanupItems: [CleanupItem] = []
    @State private var selectedIds: Set<String> = []
    @State private var selectedCategory: String = "all"
    @State private var showConfirmAlert = false
    @State private var statusMessage: String?

    public init() {}

    private var filteredItems: [CleanupItem] {
        if selectedCategory == "all" { return cleanupItems }
        return cleanupItems.filter { $0.category == selectedCategory }
    }

    public var body: some View {
        Form {
            // 1. 刮削白名单目录
            Section {
                VStack(alignment: .leading, spacing: 8) {
                    Text("仅在以下白名单目录中的视频才会自动刮削日本商业元数据与剧照；未配置则自动刮削保持静默。")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    HStack {
                        TextField("例如：/115open/文件/日本电影", text: $newPath)
                            .textFieldStyle(.roundedBorder)

                        Button("添加") {
                            let p = newPath.trimmingCharacters(in: .whitespacesAndNewlines)
                            if !p.isEmpty && !whitelist.contains(p) {
                                whitelist.append(p)
                                newPath = ""
                            }
                        }
                        .disabled(newPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }

                    if !whitelist.isEmpty {
                        ForEach(Array(whitelist.enumerated()), id: \.offset) { idx, path in
                            HStack {
                                Image(systemName: "folder")
                                    .foregroundStyle(.indigo)
                                Text(path)
                                    .font(.subheadline)
                                Spacer()
                                Button(role: .destructive) {
                                    whitelist.remove(at: idx)
                                } label: {
                                    Image(systemName: "trash")
                                        .foregroundStyle(.red)
                                }
                                .buttonStyle(.borderless)
                            }
                            .padding(.vertical, 2)
                        }
                    } else {
                        Text("当前未配置任何白名单目录")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }

                    HStack {
                        Button {
                            Task { await saveWhitelist() }
                        } label: {
                            if savingWhitelist {
                                ProgressView()
                            } else {
                                Label("保存白名单设置", systemImage: "checkmark")
                            }
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)

                        if let notice = whitelistNotice {
                            Text(notice)
                                .font(.caption2)
                                .foregroundStyle(.green)
                        }
                    }
                    .padding(.top, 4)
                }
            } header: {
                Label("刮削目录白名单限定", systemImage: "shield.lefthalf.filled")
            }

            // 2. 垃圾广告扫描与清洗
            Section {
                VStack(alignment: .leading, spacing: 10) {
                    Text("排查离线下载残留的宣传文本（*.txt）、广告链接（*.url）、广告海报图与小于25秒的片头广告切片。")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    HStack {
                        Button {
                            Task { await startScan() }
                        } label: {
                            if scanning {
                                ProgressView()
                            } else {
                                Label("开始排查垃圾文件", systemImage: "magnifyingglass")
                            }
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                        .disabled(scanning)

                        if !cleanupItems.isEmpty {
                            Spacer()
                            Button(role: .destructive) {
                                showConfirmAlert = true
                            } label: {
                                Label("清理 (\(selectedIds.count))", systemImage: "trash.fill")
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .disabled(selectedIds.isEmpty || deleting)
                        }
                    }

                    if let msg = statusMessage {
                        Text(msg)
                            .font(.footnote)
                            .foregroundStyle(.green)
                    }
                }

                if !cleanupItems.isEmpty {
                    Picker("分类", selection: $selectedCategory) {
                        Text("全部 (\(cleanupItems.count))").tag("all")
                        Text("文本 (\(countCategory("text")))").tag("text")
                        Text("图片 (\(countCategory("image")))").tag("image")
                        Text("片头视频 (\(countCategory("video")))").tag("video")
                    }
                    .pickerStyle(.segmented)

                    HStack {
                        Button(selectedIds.count == filteredItems.count ? "取消全选" : "全选当前") {
                            if selectedIds.count == filteredItems.count {
                                selectedIds.removeAll()
                            } else {
                                selectedIds = Set(filteredItems.map { $0.assetId })
                            }
                        }
                        .font(.caption)
                        Spacer()
                    }

                    ForEach(filteredItems) { item in
                        HStack(alignment: .top, spacing: 10) {
                            Button {
                                if selectedIds.contains(item.assetId) {
                                    selectedIds.remove(item.assetId)
                                } else {
                                    selectedIds.insert(item.assetId)
                                }
                            } label: {
                                Image(systemName: selectedIds.contains(item.assetId) ? "checkmark.circle.fill" : "circle")
                                    .foregroundStyle(selectedIds.contains(item.assetId) ? .indigo : .secondary)
                                    .font(.title3)
                            }
                            .buttonStyle(.plain)
                            .padding(.top, 2)

                            VStack(alignment: .leading, spacing: 3) {
                                HStack {
                                    Text(item.category == "text" ? "文本" : item.category == "image" ? "图片" : "视频")
                                        .font(.system(size: 9, weight: .bold))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 2)
                                        .background(categoryColor(item.category).opacity(0.18))
                                        .foregroundStyle(categoryColor(item.category))
                                        .clipShape(RoundedRectangle(cornerRadius: 3))

                                    Text(item.fileName)
                                        .font(.subheadline)
                                        .lineLimit(1)
                                }

                                Text(item.reason)
                                    .font(.caption2)
                                    .foregroundStyle(.orange)

                                Text(formatBytes(item.sizeBytes))
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 2)
                    }
                }
            } header: {
                Label("垃圾文件治理与安全清洗", systemImage: "trash")
            }
        }
        .navigationTitle("治理与清理中心")
        .navigationBarTitleDisplayMode(.inline)
        .alert("确认清理", isPresented: $showConfirmAlert) {
            Button("彻底删除", role: .destructive) {
                Task { await executeCleanup() }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("确定要从资料库与存储中彻底清理选中的 \(selectedIds.count) 个文件吗？此操作无法撤销。")
        }
        .task {
            whitelist = (try? await appModel.api.getScrapeWhitelist()) ?? []
        }
    }

    private func countCategory(_ cat: String) -> Int {
        cleanupItems.filter { $0.category == cat }.count
    }

    private func categoryColor(_ cat: String) -> Color {
        switch cat {
        case "text": return .orange
        case "image": return .green
        case "video": return .red
        default: return .secondary
        }
    }

    private func saveWhitelist() async {
        savingWhitelist = true
        whitelistNotice = nil
        do {
            try await appModel.api.setScrapeWhitelist(paths: whitelist)
            whitelistNotice = "保存成功！"
        } catch {
            whitelistNotice = "保存失败: \(error.localizedDescription)"
        }
        savingWhitelist = false
    }

    private func startScan() async {
        scanning = true
        statusMessage = nil
        cleanupItems = []
        selectedIds.removeAll()
        do {
            let items = try await appModel.api.scanCleanupItems()
            cleanupItems = items
            selectedIds = Set(items.map { $0.assetId })
            if items.isEmpty {
                statusMessage = "未检测到匹配的牛皮癣文件或广告片头。"
            }
        } catch {
            statusMessage = "扫描失败: \(error.localizedDescription)"
        }
        scanning = false
    }

    private func executeCleanup() async {
        deleting = true
        statusMessage = nil
        do {
            let report = try await appModel.api.deleteCleanupItems(assetIds: Array(selectedIds))
            let deletedCount = report.deletedCount
            let freed = formatBytes(report.freedBytes)
            cleanupItems.removeAll { selectedIds.contains($0.assetId) }
            selectedIds.removeAll()
            statusMessage = "已彻底清理 \(deletedCount) 个文件，释放 \(freed)！"
        } catch {
            statusMessage = "清理失败: \(error.localizedDescription)"
        }
        deleting = false
    }

    private func formatBytes(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }
}
