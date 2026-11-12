import Foundation

extension PMLAPIClient {
    // MARK: - Telegram 对话与订阅 API

    struct TelegramDialogsResponse: Decodable {
        let ok: Bool
        let folders: [TelegramNativeFolder]?
        let dialogs: [TelegramNativeDialog]?
        let error: String?
    }

    struct TelegramSubscriptionsResponse: Decodable {
        let ok: Bool
        let subscribedIds: [Int64]?
        let error: String?

        enum CodingKeys: String, CodingKey {
            case ok, error
            case subscribedIds = "subscribed_ids"
        }
    }

    struct TelegramToggleSubResponse: Decodable {
        let ok: Bool
        let chatId: Int64?
        let isSubscribed: Bool?
        let config: TelegramNativeChannelConfig?
        let error: String?

        enum CodingKeys: String, CodingKey {
            case ok, error, config
            case chatId = "chat_id"
            case isSubscribed = "is_subscribed"
        }
    }

    struct TelegramUpdateConfigResponse: Decodable {
        let ok: Bool
        let chatId: Int64?
        let config: TelegramNativeChannelConfig?
        let error: String?

        enum CodingKeys: String, CodingKey {
            case ok, error, config
            case chatId = "chat_id"
        }
    }

    struct TelegramMessagesResponse: Decodable {
        let ok: Bool
        let messages: [TelegramNativeMessage]?
        let error: String?
    }

    /// 获取所有对话列表及文件夹分类（包含 `is_subscribed` 标识）
    func telegramUserDialogs() async throws -> (folders: [TelegramNativeFolder], dialogs: [TelegramNativeDialog]) {
        let resp: TelegramDialogsResponse = try await command("telegram_user_list_dialogs", arguments: [:])
        guard resp.ok else {
            throw APIError.server(resp.error ?? "获取 Telegram 对话列表失败")
        }
        return (resp.folders ?? [], resp.dialogs ?? [])
    }

    /// 获取已订阅频道 ID 列表
    func telegramUserSubscriptions() async throws -> [Int64] {
        let resp: TelegramSubscriptionsResponse = try await command("telegram_user_subscriptions", arguments: [:])
        guard resp.ok else {
            throw APIError.server(resp.error ?? "获取订阅列表失败")
        }
        return resp.subscribedIds ?? []
    }

    /// 切换某个频道的订阅状态
    func telegramToggleSubscription(chatId: Int64) async throws -> Bool {
        let resp: TelegramToggleSubResponse = try await command("telegram_user_toggle_subscription", arguments: ["chat_id": chatId])
        guard resp.ok else {
            throw APIError.server(resp.error ?? "切换订阅失败")
        }
        return resp.isSubscribed ?? false
    }

    /// 更新频道订阅过滤配置（视频、图片、离线下载链接、评论区穿透、防广告、防重复）
    func telegramUpdateSubscriptionConfig(chatId: Int64, config: TelegramNativeChannelConfig) async throws -> TelegramNativeChannelConfig {
        let resp: TelegramUpdateConfigResponse = try await command("telegram_user_update_subscription", arguments: [
            "chat_id": chatId,
            "config": [
                "show_videos": config.showVideos,
                "show_photos": config.showPhotos,
                "show_links": config.showLinks,
                "penetrate_comments": config.penetrateComments,
                "filter_ads": config.filterAds,
                "filter_duplicates": config.filterDuplicates
            ]
        ])
        guard resp.ok, let cfg = resp.config else {
            throw APIError.server(resp.error ?? "更新频道配置失败")
        }
        return cfg
    }

    /// 获取指定会话的过滤后纯净消息流
    func telegramUserMessages(
        chatId: Int64,
        offsetId: Int64 = 0,
        limit: Int = 30,
        filterAds: Bool = true,
        filterDuplicates: Bool = true,
        config: TelegramNativeChannelConfig? = nil
    ) async throws -> [TelegramNativeMessage] {
        var args: [String: Any] = [
            "chat_id": chatId,
            "offset_id": offsetId,
            "limit": limit,
            "filter_ads": filterAds,
            "filter_duplicates": filterDuplicates
        ]
        if let config = config {
            args["show_videos"] = config.showVideos
            args["show_photos"] = config.showPhotos
            args["show_links"] = config.showLinks
            args["penetrate_comments"] = config.penetrateComments
        }
        let resp: TelegramMessagesResponse = try await command("telegram_user_list_messages", arguments: args)
        guard resp.ok else {
            throw APIError.server(resp.error ?? "获取 Telegram 消息失败")
        }
        return resp.messages ?? []
    }

    /// 获取视频的原画 206 Range 流媒体直放 URL
    func telegramUserStreamURL(chatId: Int64, messageId: Int64) -> URL? {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else {
            return nil
        }
        components.path = "/api/telegram-user-stream/\(chatId)/\(messageId)"
        return components.url
    }

    /// 获取图片/视频缩略图 URL
    func telegramUserThumbURL(chatId: Int64, messageId: Int64) -> URL? {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else {
            return nil
        }
        components.path = "/api/telegram-user-thumb/\(chatId)/\(messageId)"
        return components.url
    }
}
