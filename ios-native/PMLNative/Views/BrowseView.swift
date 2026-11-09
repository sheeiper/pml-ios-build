import SwiftUI

/// 浏览页只展示媒体源里配置好的四个目录。
struct BrowseView: View {
    @EnvironmentObject private var model: AppModel

    @State private var status: LibraryStatus?
    @State private var roots: [LibraryRoot] = []
    @State private var config: DirectoryConfigPayload?
    @State private var loading = false
    @State private var checking = false
    @State private var notice = ""
    @State private var errorText = ""

    var body: some View {
        NavigationStack {
            List {
                Section("资料库") {
                    if let config {
                        if let path = config.recentPath, let root = root(for: path) {
                            NavigationLink {
                                DirectoryView(root: root, path: path)
                            } label: {
                                slot("最近添加", path, "clock.arrow.circlepath")
                            }
                        }
                        if let path = config.pendingPath, !path.isEmpty {
                            NavigationLink {
                                PendingCurationView()
                            } label: {
                                slot("待整理", path, "tray.and.arrow.down")
                            }
                        }
                        if let path = config.vaultPath, let root = root(for: path) {
                            NavigationLink {
                                DirectoryView(root: root, path: path)
                            } label: {
                                slot("已整理", path, "folder.fill")
                            }
                        }
                        if let path = config.comicsPath, let root = root(for: path) {
                            NavigationLink {
                                DirectoryView(root: root, path: path)
                            } label: {
                                slot("漫画", path, "book.closed.fill")
                            }
                        }
                        if config.recentPath == nil && config.vaultPath == nil && config.comicsPath == nil {
                            if roots.isEmpty {
                                emptyHint
                            } else {
                                ForEach(roots) { root in
                                    NavigationLink {
                                        DirectoryView(root: root, path: root.remotePath)
                                    } label: {
                                        slot(root.remotePath, root.remotePath, "folder")
                                    }
                                }
                            }
                        }
                    } else if loading {
                        ProgressView()
                    } else {
                        emptyHint
                    }
                }

                if !(status?.connected ?? false) {
                    Section {
                        NavigationLink {
                            SettingsView()
                        } label: {
                            Label("去设置连接媒体源", systemImage: "gearshape")
                        }
                        if let status {
                            Text(status.message)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }

                if !notice.isEmpty {
                    Section {
                        Text(notice).font(.caption).foregroundStyle(.secondary)
                    }
                }

                if !errorText.isEmpty {
                    Section {
                        Text(errorText).font(.footnote).foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("浏览")
            .task { await refresh() }
            .refreshable { await refreshAndCheckProvider() }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task { await refreshAndCheckProvider() }
                    } label: {
                        if checking {
                            ProgressView()
                        } else {
                            Image(systemName: "arrow.clockwise")
                        }
                    }
                    .disabled(checking)
                    .accessibilityLabel("检查 CD2 更新")
                }
            }
        }
    }

    private var emptyHint: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("还没有配置目录")
            Text("请到「设置 → 媒体源」连接 CD2，并选择最近添加、待整理、已整理、漫画四个目录。")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
    }

    private func slot(_ title: String, _ path: String, _ symbol: String) -> some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                Text(path)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
        } icon: {
            Image(systemName: symbol)
        }
    }

    private func root(for path: String) -> LibraryRoot? {
        roots
            .filter { path == $0.remotePath || path.hasPrefix($0.remotePath + "/") }
            .sorted { $0.remotePath.count > $1.remotePath.count }
            .first
            ?? roots.first(where: { $0.kind == "media" })
            ?? roots.first
    }

    /// Single refresh path: provider check first, then authoritative reads.
    private func refreshAndCheckProvider() async {
        guard let client = model.apiClient, !checking else { return }
        checking = true
        errorText = ""
        do {
            let outcomes = try await client.checkDomainRootsForUpdates()
            await refresh()
            notice = outcomes.refreshNotice
        } catch {
            if !pmlIsCancellation(error) { errorText = error.localizedDescription }
        }
        checking = false
    }

    private func refresh() async {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = ""
        do {
            var latest = try await client.libraryStatus()
            if !latest.connected && latest.configured {
                latest = (try? await client.restoreConnection()) ?? latest
            }
            status = latest
            roots = try await client.libraryRoots()
            config = try await client.directoryConfig()
        } catch {
            if !pmlIsCancellation(error) {
                errorText = error.localizedDescription
            }
        }
        loading = false
    }
}
