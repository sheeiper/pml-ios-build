import SwiftUI

enum AppTab: Hashable {
    case home
    case browse
    case shorts
    case creators
    case profile
}

struct RootTabView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var selection: AppTab = .home

    var body: some View {
        // 使用系统 TabView：使用 iOS 26 SDK 构建后，TabBar 会自动采用 Liquid Glass 悬浮样式。
        TabView(selection: $selection) {
            HomeView()
                .tabItem { Label("首页", systemImage: "house.fill") }
                .tag(AppTab.home)

            BrowseView()
                .tabItem { Label("浏览", systemImage: "folder.fill") }
                .tag(AppTab.browse)

            ShortVideosView()
                .tabItem { Label("短视频", systemImage: "play.rectangle.fill") }
                .tag(AppTab.shorts)

            AuthorsView()
                .tabItem { Label("创作者", systemImage: "person.2.fill") }
                .tag(AppTab.creators)

            NavigationStack {
                AppCenterView()
            }
            .tabItem { Label("应用中心", systemImage: "square.grid.2x2.fill") }
            .tag(AppTab.profile)
        }
        .id(model.settingsRevision)
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            Task { await model.reconnectIfNeeded() }
        }
    }
}

struct PlaceholderScreen: View {
    let title: String
    let subtitle: String

    var body: some View {
        NavigationStack {
            VStack(spacing: 14) {
                Text(title)
                    .font(.largeTitle.weight(.bold))
                Text(subtitle)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(32)
            .navigationTitle(title)
        }
    }
}

struct HomePlaceholderView: View {
    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text("媒体源连接后，这里会显示新增文件、常看创作者与漫画馆藏。")
                        .font(.callout)
                        .foregroundStyle(.secondary)

                    HStack(spacing: 14) {
                        statCard(title: "媒体", value: "—", symbol: "photo.on.rectangle")
                        statCard(title: "漫画", value: "—", symbol: "book.closed.fill")
                        statCard(title: "创作者", value: "—", symbol: "person.2.fill")
                    }
                }
                .padding(20)
            }
            .navigationTitle("首页")
        }
    }

    private func statCard(title: String, value: String, symbol: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Image(systemName: symbol)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(.tint)
            Text(value)
                .font(.title2.weight(.semibold))
            Text(title)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(16)
        .pmlGlassCard(cornerRadius: 20)
    }
}

struct ProfilePlaceholderView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            List {
                Section {
                    VStack(spacing: 8) {
                        Image(systemName: "play.rectangle.on.rectangle")
                            .font(.system(size: 36, weight: .semibold))
                            .foregroundStyle(.tint)
                        Text("私人媒体库").font(.headline)
                        Text("版本 \(appVersion)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 10)
                }

                Section {
                    NavigationLink {
                        AppCenterView()
                    } label: {
                        Label("应用中心", systemImage: "square.grid.2x2.fill")
                    }
                }
            }
            .navigationTitle("我的")
        }
    }

    private var appVersion: String {
        (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "2.0.0"
    }
}

struct AppCenterView: View {
    private enum Destination: String, Identifiable {
        case pending, history, favorites, duplicates, scan, settings
        var id: String { rawValue }
    }

    private let columns = [GridItem(.adaptive(minimum: 150), spacing: 12)]
    @State private var destination: Destination?

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 12) {
                tile("待整理", "tray.and.arrow.down.fill", .pending)
                tile("播放记录", "clock.arrow.circlepath", .history)
                tile("收藏", "heart.fill", .favorites)
                tile("重复文件", "doc.on.doc.fill", .duplicates)
                tile("扫描中心", "arrow.clockwise.circle.fill", .scan)
                tile("设置", "gearshape.fill", .settings)
            }
            .padding(16)
        }
        .navigationTitle("应用中心")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(item: $destination) { dest in
            switch dest {
            case .pending: PendingCurationView()
            case .history: HistoryView()
            case .favorites: FavoritesView()
            case .duplicates: DuplicatesView()
            case .scan: ScanCenterView()
            case .settings: SettingsView()
            }
        }
    }

    private func tile(_ title: String, _ symbol: String, _ dest: Destination) -> some View {
        Button {
            destination = dest
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                Image(systemName: symbol)
                    .font(.system(size: 20, weight: .semibold))
                    .foregroundStyle(.tint)
                Text(title).font(.subheadline.weight(.semibold))
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, minHeight: 96, alignment: .leading)
            .padding(14)
            .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            .pmlGlassCard(cornerRadius: 18)
        }
        .buttonStyle(PressableTileStyle())
    }
}

struct PressableTileStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.94 : 1)
            .opacity(configuration.isPressed ? 0.82 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}
