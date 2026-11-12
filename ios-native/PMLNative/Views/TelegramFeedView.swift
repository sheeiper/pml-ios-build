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
    @State private var errorText: String?
    @State private var filterAds = true
    @State private var filterDuplicates = true
    @State private var showAllDialogsSheet = false
    @State private var showChannelFilterSheet = false
    @State private var currentConfig = TelegramNativeChannelConfig()
    
    // 播放器状态
    @State private var activeVideoTarget: VideoTarget?
    @State private var activeImageTarget: ImageTarget?
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
                if selectedDialog != nil {
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
                ScrollView {
                    LazyVStack(spacing: 16) {
                        ForEach(messages) { msg in
                            messageCard(msg)
                        }
                    }
                    .padding(16)
                }
                .refreshable {
                    if let d = selectedDialog {
                        await loadMessages(chatId: d.id)
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
                offlineLinksSection(links)
            }

            // 4. 评论区穿透资源盒子 (正片/网盘)
            if let replies = msg.repliesResources, !replies.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 4) {
                        Image(systemName: "bubble.left.and.bubble.right.fill")
                            .font(.caption)
                        Text("评论区精选资源 (\(replies.count))")
                            .font(.caption.weight(.bold))
                    }
                    .foregroundStyle(.blue)

                    ForEach(replies) { rep in
                        VStack(alignment: .leading, spacing: 4) {
                            if rep.isVideo == true, let rVideo = rep.video {
                                videoPreviewRow(rep.id, video: rVideo)
                            } else if rep.hasPhoto == true {
                                photoPreviewRow(rep.id)
                            }
                            if let rText = rep.text, !rText.isEmpty {
                                HStack {
                                    Text(rText)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(3)
                                    Spacer()
                                    Button {
                                        UIPasteboard.general.string = rText
                                        triggerToast("已复制评论区内容")
                                    } label: {
                                        Image(systemName: "doc.on.doc")
                                            .font(.caption)
                                    }
                                }
                            }
                        }
                        .padding(8)
                        .background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
                .padding(10)
                .background(Color.blue.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .padding(14)
        .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 16))
    }

    // MARK: - 离线下载资源列表卡片
    private func offlineLinksSection(_ links: [TelegramNativeOfflineLink]) -> some View {
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
                            UIPasteboard.general.string = lk.copyText
                            triggerToast("⚡ 已复制名称与离线链接！")
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "bolt.fill")
                                Text("复制名称+链接")
                            }
                            .font(.caption.weight(.bold))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(Color.purple, in: Capsule())
                            .foregroundStyle(.white)
                        }

                        Button {
                            UIImpactFeedbackGenerator(style: .light).impactOccurred()
                            let linkWithCode = lk.url + (lk.code.map { " 提取码: " + $0 } ?? "")
                            UIPasteboard.general.string = linkWithCode
                            triggerToast("📋 已复制下载链接")
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "doc.on.doc")
                                Text("仅复制链接")
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
        do {
            let msgs = try await client.telegramUserMessages(
                chatId: chatId,
                offsetId: 0,
                limit: 50,
                filterAds: currentConfig.filterAds,
                filterDuplicates: currentConfig.filterDuplicates,
                config: currentConfig
            )
            messages = msgs
        } catch {
            errorText = error.localizedDescription
        }
        loadingMessages = false
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
}
