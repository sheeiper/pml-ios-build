import SwiftUI
import AVKit

/// Telegram 订阅流视图：
/// 专为 iOS 打造的纯净媒体消费中心，只显示用户在 Web/App 端订阅的精选频道。
/// 具备自动防广告过滤、相册合集聚合展示、评论区资源穿透与手势视频播放器联动。
struct TelegramFeedView: View {
    @EnvironmentObject private var model: AppModel
    
    @State private var dialogs: [TelegramNativeDialog] = []
    @State private var subscribedDialogs: [TelegramNativeDialog] = []
    @State private var selectedDialog: TelegramNativeDialog?
    @State private var messages: [TelegramNativeMessage] = []
    
    @State private var loadingDialogs = false
    @State private var loadingMessages = false
    @State private var loadingOlder = false
    @State private var hasMoreOlder = true
    @State private var errorText: String?
    @State private var filterAds = true
    @State private var filterDuplicates = true
    @State private var showAllDialogsSheet = false
    @State private var showChannelFilterSheet = false
    @State private var currentConfig = TelegramNativeChannelConfig()
    
    // 播放器状态
    @State private var activeVideoTarget: VideoTarget?
    @State private var activeImageTarget: ImageTarget?
    @State private var activeCommentsMsg: TelegramNativeMessage?
    @State private var copyToast: String?
    
    struct VideoTarget: Identifiable {
        let id = UUID()
        let url: URL
        let title: String
    }

    struct ImageTarget: Identifiable {
        let id = UUID()
        let url: URL
    }

    var body: some View {
        VStack(spacing: 0) {
            if loadingDialogs && dialogs.isEmpty {
                ProgressView("正在同步 Telegram 订阅...")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if subscribedDialogs.isEmpty {
                emptySubscribedView
            } else {
                mainContentView
            }
        }
        .navigationTitle("Telegram 订阅")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .topBarTrailing) {
                if let dialog = selectedDialog {
                    Button {
                        Task {
                            await markSelectedDialogAsRead()
                        }
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "checkmark.circle")
                            if let count = dialog.unreadCount, count > 0 {
                                Text("\(count)")
                                    .font(.caption2.bold())
                            }
                        }
                    }
                    .accessibilityLabel("一键已读")

                    Button {
                        if let cfg = selectedDialog?.config {
                            currentConfig = cfg
                        }
                        showChannelFilterSheet = true
                    } label: {
                        Image(systemName: "slider.horizontal.3")
                    }
                    .accessibilityLabel("过滤设置")
                }

                Button {
                    showAllDialogsSheet = true
                } label: {
                    Image(systemName: "list.star")
                }
                .accessibilityLabel("频道订阅管理")
            }
        }
        .overlay(alignment: .top) {
            if let copyToast {
                Text(copyToast)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .background(.black.opacity(0.8), in: Capsule())
                    .padding(.top, 10)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .fullScreenCover(item: $activeVideoTarget) { target in
            GestureVideoPlayerSheet(
                url: target.url,
                title: target.title,
                sourceLabel: "Telegram 原画"
            )
        }
        .sheet(item: $activeImageTarget) { target in
            NavigationStack {
                AsyncImage(url: target.url) { phase in
                    switch phase {
                    case .success(let image):
                        image
                            .resizable()
                            .scaledToFit()
                    case .failure:
                        Image(systemName: "photo")
                            .font(.largeTitle)
                            .foregroundStyle(.secondary)
                    default:
                        ProgressView()
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.black)
                .navigationTitle("图片预览")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("关闭") { activeImageTarget = nil }
                    }
                }
            }
        }
        .sheet(item: $activeCommentsMsg) { msg in
            TelegramCommentsSheet(
                message: msg,
                onPlayVideo: { url, title in
                    activeCommentsMsg = nil
                    activeVideoTarget = VideoTarget(url: url, title: title)
                },
                onShowImage: { url in
                    activeCommentsMsg = nil
                    activeImageTarget = ImageTarget(url: url)
                },
                onToast: { t in
                    triggerToast(t)
                }
            )
        }
        .sheet(isPresented: $showAllDialogsSheet) {
            allDialogsManagementSheet
        }
        .sheet(isPresented: $showChannelFilterSheet) {
            channelFilterSheet
        }
        .task {
            await loadDialogsAndSubscriptions()
        }
    }

    // MARK: - 主内容区（频道水平胶囊栏 + 垂直消息流）

    private var mainContentView: some View {
        VStack(spacing: 0) {
            // 顶部横向频道切换栏
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(subscribedDialogs) { d in
                        let isSelected = selectedDialog?.id == d.id
                        Button {
                            selectedDialog = d
                            currentConfig = d.config ?? TelegramNativeChannelConfig()
                            Task { await loadMessages(chatId: d.id) }
                        } label: {
                            HStack(spacing: 6) {
                                Circle()
                                    .fill(isSelected ? Color.white : Color.accentColor)
                                    .frame(width: 8, height: 8)
                                Text(d.title)
                                    .font(.subheadline.weight(isSelected ? .bold : .medium))
                                    .lineLimit(1)
                            }
                            .padding(.horizontal, 14)
                            .padding(.vertical, 8)
                            .background(
                                isSelected ? Color.accentColor : Color(uiColor: .secondarySystemFill),
                                in: Capsule()
                            )
                            .foregroundStyle(isSelected ? Color.white : Color.primary)
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
            }
            .background(Color(uiColor: .systemBackground))
            .overlay(alignment: .bottom) {
                Divider()
            }

            // 垂直消息流
            if loadingMessages && messages.isEmpty {
                ProgressView("加载消息与资源...")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if messages.isEmpty {
                VStack(spacing: 12) {
                    Image(systemName: "tray")
                        .font(.system(size: 40))
                        .foregroundStyle(.secondary)
                    Text("当前频道暂无消息或均被过滤")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollViewReader { scrollProxy in
                    ScrollView {
                        LazyVStack(spacing: 16) {
                            // 顶部：向上翻看加载更早历史消息
                            if hasMoreOlder && !messages.isEmpty {
                                if loadingOlder {
                                    HStack(spacing: 8) {
                                        ProgressView()
                                            .controlSize(.small)
                                        Text("正在加载更早历史消息...")
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                    .padding(.vertical, 8)
                                } else {
                                    Button {
                                        Task { await loadOlderMessages() }
                                    } label: {
                                        HStack(spacing: 4) {
                                            Image(systemName: "arrow.up.circle")
                                            Text("点击或往上翻加载更早消息")
                                        }
                                        .font(.caption.weight(.medium))
                                        .foregroundStyle(.secondary)
                                        .padding(.vertical, 6)
                                        .padding(.horizontal, 14)
                                        .background(Color(uiColor: .secondarySystemBackground), in: Capsule())
                                    }
                                    .padding(.top, 4)
                                    .onAppear {
                                        Task { await loadOlderMessages() }
                                    }
                                }
                            }

                            ForEach(messages) { msg in
                                messageCard(msg)
                                    .id(msg.id)
                            }

                            // 底部锚点：保证最新的内容在最下方
                            Color.clear
                                .frame(height: 1)
                                .id("BOTTOM_MSG_ANCHOR")
                        }
                        .padding(16)
                    }
                    .refreshable {
                        if let d = selectedDialog {
                            await loadMessages(chatId: d.id)
                        }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        Button {
                            withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
                                scrollProxy.scrollTo("BOTTOM_MSG_ANCHOR", anchor: .bottom)
                            }
                        } label: {
                            Image(systemName: "arrow.down.circle.fill")
                                .font(.system(size: 34))
                                .foregroundStyle(Color.accentColor.opacity(0.85))
                                .background(Circle().fill(Color(uiColor: .systemBackground)))
                                .shadow(color: .black.opacity(0.18), radius: 4, x: 0, y: 2)
                        }
                        .padding(.trailing, 20)
                        .padding(.bottom, 16)
                    }
                    .onChange(of: selectedDialog?.id) { _ in
                        scrollProxy.scrollTo("BOTTOM_MSG_ANCHOR", anchor: .bottom)
                    }
                    .onChange(of: messages.count) { _ in
                        if !loadingOlder {
                            scrollProxy.scrollTo("BOTTOM_MSG_ANCHOR", anchor: .bottom)
                        }
                    }
                    .onAppear {
                        scrollProxy.scrollTo("BOTTOM_MSG_ANCHOR", anchor: .bottom)
                    }
                }
            }
        }
    }

    // MARK: - 消息卡片组件

    private func messageCard(_ msg: TelegramNativeMessage) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            // 头部：发送者与时间
            HStack {
                if let d = selectedDialog {
                    Text(d.title)
                        .font(.caption.weight(.bold))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if msg.isAlbum == true, let count = msg.albumCount {
                    Text("🎞️ 合集 \(count) 项")
                        .font(.caption2.weight(.bold))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.blue.opacity(0.15), in: Capsule())
                        .foregroundStyle(.blue)
                }
                if let date = msg.date {
                    Text(formatDate(date))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            // 1. 合集媒体列表
            if msg.isAlbum == true, let mediaList = msg.mediaList, !mediaList.isEmpty {
                VStack(spacing: 8) {
                    ForEach(mediaList) { item in
                        if item.isVideo == true, let video = item.video {
                            videoPreviewRow(item.id, video: video)
                        } else if item.hasPhoto == true {
                            photoPreviewRow(item.id)
                        }
                    }
                }
            }
            // 2. 单独视频
            else if msg.isVideo == true, let video = msg.video {
                videoPreviewRow(msg.id, video: video)
            }
            // 3. 单独图片
            else if msg.hasPhoto == true {
                photoPreviewRow(msg.id)
            }

            // 正文描述
            if let text = msg.text, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(text)
                    .font(.body)
                    .foregroundStyle(.primary)
                    .lineLimit(8)
            }

            // 离线下载资源盒子 (磁力/网盘/电驴 + 快速复制)
            if let links = msg.offlineLinks, !links.isEmpty {
                offlineLinksSection(links, parentText: msg.text)
            }

            // 评论区入口 (按需点击展开专属讨论抽屉)
            if let rCount = msg.repliesCount, rCount > 0 {
                Button {
                    activeCommentsMsg = msg
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "bubble.left.and.bubble.right.fill")
                            .font(.caption)
                        Text("\(rCount) 条评论")
                            .font(.subheadline.weight(.semibold))
                        Spacer()
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(.tertiary)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(Color.blue.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
                    .foregroundStyle(.blue)
                }
            }
        }
        .padding(14)
        .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 16))
    }

    // MARK: - 离线下载资源列表卡片
    private func offlineLinksSection(_ links: [TelegramNativeOfflineLink], parentText: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "arrow.down.circle.fill")
                    .font(.subheadline)
                    .foregroundStyle(.purple)
                Text("离线下载资源 (\(links.count))")
                    .font(.subheadline.weight(.bold))
                    .foregroundStyle(.purple)
                Spacer()
                Text("快速导入 115 / 迅雷")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }

            ForEach(links) { lk in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 6) {
                        Text(linkBadgeTitle(lk.type))
                            .font(.caption2.weight(.bold))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(linkBadgeColor(lk.type).opacity(0.18), in: Capsule())
                            .foregroundStyle(linkBadgeColor(lk.type))

                        if lk.source == "comment" {
                            Text("评论区")
                                .font(.caption2.weight(.medium))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 2)
                                .background(Color.blue.opacity(0.12), in: Capsule())
                                .foregroundStyle(.blue)
                        }

                        if let code = lk.code, !code.isEmpty {
                            Text("提取码: \(code)")
                                .font(.caption2.weight(.bold).monospaced())
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.orange.opacity(0.15), in: Capsule())
                                .foregroundStyle(.orange)
                        }
                        Spacer()
                    }

                    Text(lk.name)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)

                    Text(lk.url)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    HStack(spacing: 8) {
                        Button {
                            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                            let clean = lk.textWithoutLinks ?? removeURLsFromText(parentText ?? lk.name)
                            let finalText = (lk.code != nil && !clean.contains(lk.code!)) ? "\(clean)\n提取码: \(lk.code!)" : clean
                            UIPasteboard.general.string = finalText
                            triggerToast("📄 已复制信息文本（无链接）")
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "doc.text")
                                Text("复制文本")
                            }
                            .font(.caption.weight(.bold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Color.blue, in: Capsule())
                            .foregroundStyle(.white)
                        }

                        Button {
                            UIImpactFeedbackGenerator(style: .light).impactOccurred()
                            let isDirect = lk.url.hasPrefix("magnet:") || lk.url.hasPrefix("ed2k:") || lk.url.hasPrefix("thunder:")
                            let linkStr = (!isDirect && lk.code != nil) ? "\(lk.url) 提取码: \(lk.code!)" : lk.url
                            UIPasteboard.general.string = linkStr
                            triggerToast("🔗 已复制下载链接")
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "link")
                                Text("复制链接")
                            }
                            .font(.caption.weight(.medium))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(Color(uiColor: .tertiarySystemFill), in: Capsule())
                            .foregroundStyle(.primary)
                        }
                    }
                    .padding(.top, 2)
                }
                .padding(10)
                .background(Color(uiColor: .systemBackground), in: RoundedRectangle(cornerRadius: 10))
            }
        }
        .padding(12)
        .background(Color.purple.opacity(0.08), in: RoundedRectangle(cornerRadius: 14))
    }

    // 视频卡片行（点击直调 GestureVideoPlayerSheet）
    private func videoPreviewRow(_ msgId: Int64, video: TelegramNativeVideo) -> some View {
        Button {
            guard let chatId = selectedDialog?.id,
                  let streamURL = model.apiClient?.telegramUserStreamURL(chatId: chatId, messageId: msgId) else {
                return
            }
            activeVideoTarget = VideoTarget(url: streamURL, title: video.fileName ?? "Telegram 视频")
        } label: {
            ZStack {
                Color.black.opacity(0.85)
                    .frame(height: 180)
                    .clipShape(RoundedRectangle(cornerRadius: 12))

                // 尝试加载缩略图
                if let chatId = selectedDialog?.id,
                   let thumbURL = model.apiClient?.telegramUserThumbURL(chatId: chatId, messageId: msgId) {
                    AsyncImage(url: thumbURL) { phase in
                        if let img = phase.image {
                            img.resizable()
                                .scaledToFill()
                                .frame(height: 180)
                                .clipped()
                                .clipShape(RoundedRectangle(cornerRadius: 12))
                        }
                    }
                }

                // 播放遮罩与按钮
                Circle()
                    .fill(.ultraThinMaterial)
                    .frame(width: 54, height: 54)
                Image(systemName: "play.fill")
                    .font(.title2)
                    .foregroundStyle(.white)

                // 底部信息条
                VStack {
                    Spacer()
                    HStack {
                        if let name = video.fileName {
                            Text(name)
                                .font(.caption.weight(.medium))
                                .lineLimit(1)
                        }
                        Spacer()
                        if let dur = video.duration {
                            Text(formatDuration(dur))
                                .font(.caption2.weight(.bold))
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(.black.opacity(0.6), in: Capsule())
                        }
                        if let size = video.fileSize {
                            Text(formatFileSize(size))
                                .font(.caption2)
                        }
                    }
                    .foregroundStyle(.white)
                    .padding(8)
                    .background(.linearGradient(colors: [.clear, .black.opacity(0.7)], startPoint: .top, endPoint: .bottom))
                }
            }
        }
        .buttonStyle(.plain)
    }

    // 图片卡片行
    private func photoPreviewRow(_ msgId: Int64) -> some View {
        Button {
            guard let chatId = selectedDialog?.id,
                  let thumbURL = model.apiClient?.telegramUserThumbURL(chatId: chatId, messageId: msgId) else {
                return
            }
            activeImageTarget = ImageTarget(url: thumbURL)
        } label: {
            if let chatId = selectedDialog?.id,
               let thumbURL = model.apiClient?.telegramUserThumbURL(chatId: chatId, messageId: msgId) {
                AsyncImage(url: thumbURL) { phase in
                    switch phase {
                    case .success(let image):
                        image.resizable()
                            .scaledToFit()
                            .frame(maxHeight: 240)
                            .clipShape(RoundedRectangle(cornerRadius: 12))
                    case .failure:
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color.gray.opacity(0.3))
                            .frame(height: 120)
                            .overlay(Image(systemName: "photo"))
                    default:
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color.gray.opacity(0.2))
                            .frame(height: 120)
                            .overlay(ProgressView())
                    }
                }
            }
        }
        .buttonStyle(.plain)
    }

    // MARK: - 无订阅空状态

    private var emptySubscribedView: some View {
        VStack(spacing: 16) {
            Image(systemName: "star.slash.fill")
                .font(.system(size: 52))
                .foregroundStyle(.orange)
            Text("暂无订阅的频道")
                .font(.title2.weight(.bold))
            Text("您可以在 Web 端或点击下方按钮，将您常看的频道/群组添加至订阅列表，这里将专为您呈现纯净视频流。")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 32)

            Button {
                showAllDialogsSheet = true
            } label: {
                Label("从全部对话中订阅", systemImage: "star.fill")
                    .font(.headline)
                    .padding(.horizontal, 24)
                    .padding(.vertical, 12)
                    .background(Color.accentColor, in: Capsule())
                    .foregroundStyle(.white)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
    }

    // MARK: - 频道订阅管理弹窗

    private var allDialogsManagementSheet: some View {
        NavigationStack {
            List {
                Section("点击星标快速订阅至 iOS") {
                    ForEach(dialogs) { d in
                        HStack {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(d.title)
                                    .font(.headline)
                                    .lineLimit(1)
                                if let user = d.username, !user.isEmpty {
                                    Text("@\(user)")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            Button {
                                Task { await toggleSub(d) }
                            } label: {
                                Image(systemName: (d.isSubscribed == true) ? "star.fill" : "star")
                                    .font(.title2)
                                    .foregroundStyle((d.isSubscribed == true) ? .orange : .secondary)
                            }
                            .buttonStyle(.borderless)
                        }
                        .padding(.vertical, 4)
                    }
                }
            }
            .navigationTitle("管理订阅频道")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { showAllDialogsSheet = false }
                }
            }
        }
    }

    // MARK: - 频道订阅与过滤配置 Sheet
    private var channelFilterSheet: some View {
        NavigationStack {
            Form {
                if let d = selectedDialog {
                    Section {
                        HStack(spacing: 12) {
                            Circle()
                                .fill(Color.accentColor.opacity(0.18))
                                .frame(width: 44, height: 44)
                                .overlay {
                                    Text(String(d.title.prefix(1)).uppercased())
                                        .font(.headline.weight(.bold))
                                        .foregroundStyle(Color.accentColor)
                                }
                            VStack(alignment: .leading, spacing: 2) {
                                Text(d.title)
                                    .font(.headline)
                                Text("ID: \(d.id) · \(d.isChannel == true ? "频道" : "群组")")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                }

                Section("显示内容类型 (至少勾选一项)") {
                    Toggle(isOn: $currentConfig.showVideos) {
                        Label("🎬 视频 (Videos)", systemImage: "video.fill")
                    }
                    Toggle(isOn: $currentConfig.showPhotos) {
                        Label("📷 图片 (Photos)", systemImage: "photo.fill")
                    }
                    Toggle(isOn: $currentConfig.showLinks) {
                        Label("🧲 离线下载链接 (Offline Links)", systemImage: "arrow.down.circle.fill")
                    }
                }

                Section("高级识别与过滤引擎") {
                    Toggle(isOn: $currentConfig.penetrateComments) {
                        VStack(alignment: .leading, spacing: 2) {
                            Label("💬 开启评论区识别", systemImage: "bubble.left.and.bubble.right.fill")
                            Text("自动穿透抓取评论区里的视频、图片和离线下载链接")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }

                    Toggle(isOn: $currentConfig.filterAds) {
                        VStack(alignment: .leading, spacing: 2) {
                            Label("🛡️ 智能防广告过滤", systemImage: "shield.fill")
                            Text("自动过滤博彩、洗钱、黑产等垃圾广告")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }

                    Toggle(isOn: $currentConfig.filterDuplicates) {
                        VStack(alignment: .leading, spacing: 2) {
                            Label("♻️ 智能防刷屏消重", systemImage: "arrow.2.squarepath")
                            Text("基于文本指纹与媒体哈希折叠重复推送")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .navigationTitle("订阅过滤设置")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("取消") {
                        showChannelFilterSheet = false
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("保存并应用") {
                        Task {
                            await saveChannelConfig()
                        }
                    }
                    .bold()
                }
            }
        }
    }

    // MARK: - 数据请求与操作

    private func saveChannelConfig() async {
        guard let d = selectedDialog, let client = model.apiClient else { return }
        do {
            let updated = try await client.telegramUpdateSubscriptionConfig(chatId: d.id, config: currentConfig)
            if let idx = dialogs.firstIndex(where: { $0.id == d.id }) {
                dialogs[idx].config = updated
            }
            if let idx = subscribedDialogs.firstIndex(where: { $0.id == d.id }) {
                subscribedDialogs[idx].config = updated
            }
            selectedDialog?.config = updated
            showChannelFilterSheet = false
            triggerToast("✅ 频道过滤设置已更新")
            await loadMessages(chatId: d.id)
        } catch {
            triggerToast("保存失败: \(error.localizedDescription)")
        }
    }

    private func loadDialogsAndSubscriptions() async {
        guard let client = model.apiClient else { return }
        loadingDialogs = true
        do {
            let res = try await client.telegramUserDialogs()
            dialogs = res.dialogs
            subscribedDialogs = res.dialogs.filter { $0.isSubscribed == true }
            if selectedDialog == nil, let first = subscribedDialogs.first {
                selectedDialog = first
                currentConfig = first.config ?? TelegramNativeChannelConfig()
                await loadMessages(chatId: first.id)
            }
        } catch {
            errorText = error.localizedDescription
        }
        loadingDialogs = false
    }

    private func loadMessages(chatId: Int64) async {
        guard let client = model.apiClient else { return }
        loadingMessages = true
        messages = []
        hasMoreOlder = true
        do {
            let msgs = try await client.telegramUserMessages(
                chatId: chatId,
                offsetId: 0,
                limit: 40,
                filterAds: currentConfig.filterAds,
                filterDuplicates: currentConfig.filterDuplicates,
                config: currentConfig
            )
            // Telegram API 默认返回从新到旧 [最新, 次新, ..., 较早]
            // 反转为时间正序：[较早, ..., 次新, 最新]，使最新内容呈现在最下方
            messages = msgs.reversed()
        } catch {
            errorText = error.localizedDescription
        }
        loadingMessages = false
    }

    private func loadOlderMessages() async {
        guard let d = selectedDialog, let client = model.apiClient, !loadingOlder, hasMoreOlder else { return }
        guard let oldestMsgId = messages.map(\.id).min(), oldestMsgId > 1 else {
            hasMoreOlder = false
            return
        }
        loadingOlder = true
        do {
            let olderMsgs = try await client.telegramUserMessages(
                chatId: d.id,
                offsetId: oldestMsgId,
                limit: 30,
                filterAds: currentConfig.filterAds,
                filterDuplicates: currentConfig.filterDuplicates,
                config: currentConfig
            )
            if olderMsgs.isEmpty {
                hasMoreOlder = false
            } else {
                let existingIds = Set(messages.map(\.id))
                let newUnique = olderMsgs.filter { !existingIds.contains($0.id) }
                if newUnique.isEmpty {
                    hasMoreOlder = false
                } else {
                    // 反转后拼接到最顶部
                    messages = newUnique.reversed() + messages
                }
            }
        } catch {
            // 加载较早历史遇到网络或限流时静默容错
        }
        loadingOlder = false
    }

    private func linkBadgeTitle(_ type: String) -> String {
        switch type {
        case "magnet": return "🧲 磁力链"
        case "115": return "📦 115网盘"
        case "quark": return "⚡ 夸克网盘"
        case "baidu": return "☁️ 百度网盘"
        case "pikpak": return "🚀 PikPak"
        case "ed2k": return "🐴 ED2K电驴"
        case "thunder": return "⚡ 迅雷下载"
        case "aliyun": return "📂 阿里云盘"
        case "uc": return "🛸 UC网盘"
        case "xunlei": return "🌩️ 迅雷云盘"
        default: return "🔗 离线下载"
        }
    }

    private func linkBadgeColor(_ type: String) -> Color {
        switch type {
        case "magnet": return .orange
        case "115": return .red
        case "quark": return .green
        case "baidu": return .blue
        case "pikpak": return .purple
        case "ed2k": return .brown
        case "thunder": return .yellow
        default: return .indigo
        }
    }

    private func toggleSub(_ dialog: TelegramNativeDialog) async {
        guard let client = model.apiClient else { return }
        do {
            let isSub = try await client.telegramToggleSubscription(chatId: dialog.id)
            if let idx = dialogs.firstIndex(where: { $0.id == dialog.id }) {
                dialogs[idx].isSubscribed = isSub
            }
            subscribedDialogs = dialogs.filter { $0.isSubscribed == true }
            if isSub {
                triggerToast("已订阅 \(dialog.title)")
                if selectedDialog == nil {
                    selectedDialog = dialog
                    await loadMessages(chatId: dialog.id)
                }
            } else {
                triggerToast("已取消订阅")
            }
        } catch {
            triggerToast("操作失败: \(error.localizedDescription)")
        }
    }

    private func markSelectedDialogAsRead() async {
        guard let d = selectedDialog, let client = model.apiClient else { return }
        do {
            try await client.telegramUserMarkRead(chatId: d.id)
            if let idx = dialogs.firstIndex(where: { $0.id == d.id }) {
                dialogs[idx].unreadCount = 0
            }
            if let idx = subscribedDialogs.firstIndex(where: { $0.id == d.id }) {
                subscribedDialogs[idx].unreadCount = 0
            }
            selectedDialog?.unreadCount = 0
            triggerToast("✅ 已一键标记已读")
        } catch {
            triggerToast("标记已读失败: \(error.localizedDescription)")
        }
    }

    private func triggerToast(_ text: String) {
        withAnimation { copyToast = text }
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
            withAnimation { copyToast = nil }
        }
    }

    private func formatDate(_ dateStr: String) -> String {
        return dateStr.replacingOccurrences(of: "-", with: "/")
    }

    private func formatDuration(_ sec: Double) -> String {
        let m = Int(sec) / 60
        let s = Int(sec) % 60
        return String(format: "%02d:%02d", m, s)
    }

    private func formatFileSize(_ bytes: Int64) -> String {
        if bytes < 1024 * 1024 {
            return String(format: "%.1f KB", Double(bytes) / 1024.0)
        } else if bytes < 1024 * 1024 * 1024 {
            return String(format: "%.1f MB", Double(bytes) / (1024.0 * 1024.0))
        } else {
            return String(format: "%.2f GB", Double(bytes) / (1024.0 * 1024.0 * 1024.0))
        }
    }

    private func removeURLsFromText(_ text: String) -> String {
        var t = text
        t = t.replacingOccurrences(of: "https?://\\S+", with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "magnet:\\?\\S+", with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "ed2k://\\S+", with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "thunder://\\S+", with: "", options: .regularExpression)
        let lines = t.components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { line in
                guard !line.isEmpty else { return false }
                let isPrefixOnly = line.range(of: #"^(?:网盘|链接|下载地址|转存|下载|夸克|115|百度|迅雷|阿里)[:：\s]*$"#, options: .regularExpression) != nil
                return !isPrefixOnly
            }
        let res = lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return res.isEmpty ? text : res
    }
}

// MARK: - 专属评论区讨论抽屉 (按需点击展开)
struct TelegramCommentsSheet: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    let message: TelegramNativeMessage
    let onPlayVideo: (URL, String) -> Void
    let onShowImage: (URL) -> Void
    let onToast: (String) -> Void

    @State private var loading = true
    @State private var comments: [TelegramNativeComment] = []
    @State private var offlineLinks: [TelegramNativeOfflineLink] = []
    @State private var parentTitle: String = ""
    @State private var errorText: String?

    var body: some View {
        NavigationStack {
            Group {
                if loading && comments.isEmpty {
                    ProgressView("正在加载评论讨论...")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let errorText {
                    VStack(spacing: 12) {
                        Image(systemName: "exclamationmark.triangle")
                            .font(.system(size: 36))
                            .foregroundStyle(.orange)
                        Text(errorText)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        Button("重试") {
                            loadComments()
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    List {
                        // 1. 原帖信息卡片
                        Section {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack {
                                    Text("📌 原帖摘要")
                                        .font(.caption.weight(.bold))
                                        .foregroundStyle(.blue)
                                    Spacer()
                                    if let date = message.date {
                                        Text(date)
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                    }
                                }
                                if let text = message.text, !text.isEmpty {
                                    Text(text)
                                        .font(.subheadline)
                                        .foregroundStyle(.primary)
                                        .lineLimit(4)
                                }
                            }
                            .padding(.vertical, 4)
                        }

                        // 2. 评论区提取的离线下载资源
                        if !offlineLinks.isEmpty {
                            Section(header: Label("评论区提取资源 (\(offlineLinks.count))", systemImage: "bolt.fill").foregroundStyle(.purple)) {
                                ForEach(offlineLinks) { lk in
                                    offlineLinkCard(lk, parentText: message.text)
                                }
                            }
                        }

                        // 3. 评论讨论列表
                        Section(header: Label("评论讨论 (\(comments.count))", systemImage: "bubble.left.and.bubble.right.fill")) {
                            if comments.isEmpty {
                                Text("暂无评论内容")
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                            } else {
                                ForEach(comments) { c in
                                    commentRow(c)
                                }
                            }
                        }
                    }
                    .listStyle(.insetGrouped)
                }
            }
            .navigationTitle("评论讨论")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") {
                        dismiss()
                    }
                }
            }
            .task {
                loadComments()
            }
        }
    }

    private func loadComments() {
        guard let client = model.apiClient else { return }
        loading = true
        errorText = nil
        Task {
            do {
                let resp = try await client.telegramUserComments(
                    chatId: message.chatId,
                    messageId: message.id
                )
                await MainActor.run {
                    self.comments = resp.comments ?? []
                    self.offlineLinks = resp.offlineLinks ?? []
                    self.parentTitle = resp.parentTitle ?? ""
                    self.loading = false
                }
            } catch {
                await MainActor.run {
                    self.errorText = error.localizedDescription
                    self.loading = false
                }
            }
        }
    }

    private func offlineLinkCard(_ lk: TelegramNativeOfflineLink, parentText: String?) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(linkBadgeTitle(lk.type))
                    .font(.caption2.weight(.bold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(linkBadgeColor(lk.type).opacity(0.18), in: Capsule())
                    .foregroundStyle(linkBadgeColor(lk.type))

                if let code = lk.code, !code.isEmpty {
                    Text("提取码: \(code)")
                        .font(.caption2.weight(.bold).monospaced())
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.orange.opacity(0.15), in: Capsule())
                        .foregroundStyle(.orange)
                }
                Spacer()
            }

            Text(lk.name)
                .font(.subheadline.weight(.semibold))
                .lineLimit(2)

            Text(lk.url)
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)

            HStack(spacing: 8) {
                Button {
                    UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                    let clean = lk.textWithoutLinks ?? removeURLs(parentText ?? lk.name)
                    let textToCopy = (lk.code != nil && !clean.contains(lk.code!)) ? "\(clean)\n提取码: \(lk.code!)" : clean
                    UIPasteboard.general.string = textToCopy
                    onToast("📄 已复制文本（无链接）")
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "doc.text")
                        Text("复制文本")
                    }
                    .font(.caption.weight(.bold))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Color.blue, in: Capsule())
                    .foregroundStyle(.white)
                }

                Button {
                    UIImpactFeedbackGenerator(style: .light).impactOccurred()
                    let isDirect = lk.url.hasPrefix("magnet:") || lk.url.hasPrefix("ed2k:") || lk.url.hasPrefix("thunder:")
                    let linkStr = (!isDirect && lk.code != nil) ? "\(lk.url) 提取码: \(lk.code!)" : lk.url
                    UIPasteboard.general.string = linkStr
                    onToast("🔗 已复制下载链接")
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "link")
                        Text("复制链接")
                    }
                    .font(.caption.weight(.medium))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                    .background(Color(uiColor: .tertiarySystemFill), in: Capsule())
                    .foregroundStyle(.primary)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func commentRow(_ c: TelegramNativeComment) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Circle()
                    .fill(Color.accentColor.opacity(0.8))
                    .frame(width: 24, height: 24)
                    .overlay {
                        Text((c.senderName ?? "U").prefix(1).uppercased())
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(.white)
                    }
                Text(c.senderName ?? "Telegram 用户")
                    .font(.subheadline.weight(.semibold))
                Spacer()
                if let date = c.date {
                    Text(date.suffix(8))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }

            if let text = c.text, !text.isEmpty {
                Text(text)
                    .font(.body)
                    .foregroundStyle(.primary)
            }

            if c.isVideo == true, let video = c.video, let url = model.apiClient?.telegramUserStreamURL(chatId: c.chatId ?? message.chatId, messageId: c.id) {
                Button {
                    onPlayVideo(url, video.fileName ?? "评论区视频")
                } label: {
                    HStack {
                        Image(systemName: "play.circle.fill")
                        Text("播放视频: \(video.fileName ?? "视频")")
                    }
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.blue)
                }
            }

            if c.hasPhoto == true, let thumbURL = model.apiClient?.telegramUserThumbURL(chatId: c.chatId ?? message.chatId, messageId: c.id) {
                AsyncImage(url: thumbURL) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFit().frame(maxHeight: 160).cornerRadius(8)
                            .onTapGesture { onShowImage(thumbURL) }
                    }
                }
            }

            if let cLinks = c.offlineLinks, !cLinks.isEmpty {
                ForEach(cLinks) { cl in
                    offlineLinkCard(cl, parentText: c.text)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func removeURLs(_ text: String) -> String {
        var t = text
        t = t.replacingOccurrences(of: "https?://\\S+", with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "magnet:\\?\\S+", with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "ed2k://\\S+", with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "thunder://\\S+", with: "", options: .regularExpression)
        let lines = t.components(separatedBy: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        let res = lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return res.isEmpty ? text : res
    }

    private func linkBadgeTitle(_ type: String) -> String {
        switch type {
        case "magnet": return "🧲 磁力链"
        case "115": return "📦 115网盘"
        case "quark": return "⚡ 夸克网盘"
        case "baidu": return "☁️ 百度网盘"
        case "pikpak": return "🚀 PikPak"
        case "ed2k": return "🐴 电驴"
        case "thunder": return "⚡ 迅雷"
        case "aliyun": return "📂 阿里"
        case "uc": return "🛸 UC"
        case "xunlei": return "🌩️ 迅雷云"
        default: return "🔗 离线下载"
        }
    }

    private func linkBadgeColor(_ type: String) -> Color {
        switch type {
        case "magnet": return .red
        case "115": return .orange
        case "quark": return .blue
        case "baidu": return .teal
        case "pikpak": return .indigo
        case "ed2k": return .brown
        case "thunder", "xunlei": return .cyan
        case "aliyun": return .purple
        case "uc": return .green
        default: return .secondary
        }
    }
}
