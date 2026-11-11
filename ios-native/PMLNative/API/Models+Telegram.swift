import Foundation

// MARK: - Telegram iOS Native Models

public struct TelegramNativeFolder: Identifiable, Codable, Hashable {
    public let id: Int
    public let title: String
    public let emoticon: String?

    public init(id: Int, title: String, emoticon: String? = nil) {
        self.id = id
        self.title = title
        self.emoticon = emoticon
    }
}

public struct TelegramNativeDialog: Identifiable, Codable, Hashable {
    public let id: Int64
    public let title: String
    public let isSavedMessages: Bool?
    public let isChannel: Bool?
    public let isGroup: Bool?
    public let isUser: Bool?
    public var isSubscribed: Bool?
    public let archived: Bool?
    public let pinned: Bool?
    public let folderIds: [Int]?
    public let username: String?
    public let unreadCount: Int?
    public let lastMessage: String?
    public let date: String?

    enum CodingKeys: String, CodingKey {
        case id, title, username, archived, pinned, date
        case isSavedMessages = "is_saved_messages"
        case isChannel = "is_channel"
        case isGroup = "is_group"
        case isUser = "is_user"
        case isSubscribed = "is_subscribed"
        case folderIds = "folder_ids"
        case unreadCount = "unread_count"
        case lastMessage = "last_message"
    }
}

public struct TelegramNativeReaction: Codable, Hashable {
    public let emoji: String
    public let count: Int
}

public struct TelegramNativeVideo: Codable, Hashable {
    public let fileName: String?
    public let fileSize: Int64?
    public let duration: Double?
    public let width: Int?
    public let height: Int?
    public let mimeType: String?
    public let hasThumb: Bool?

    enum CodingKeys: String, CodingKey {
        case duration, width, height
        case fileName = "file_name"
        case fileSize = "file_size"
        case mimeType = "mime_type"
        case hasThumb = "has_thumb"
    }
}

public struct TelegramNativeMediaItem: Identifiable, Codable, Hashable {
    public let id: Int64
    public let hasPhoto: Bool?
    public let isVideo: Bool?
    public let video: TelegramNativeVideo?

    enum CodingKeys: String, CodingKey {
        case id, video
        case hasPhoto = "has_photo"
        case isVideo = "is_video"
    }
}

public struct TelegramNativeMessage: Identifiable, Codable, Hashable {
    public let id: Int64
    public let chatId: Int64
    public let date: String?
    public let text: String?
    public let isAd: Bool?
    public let hasPhoto: Bool?
    public let isVideo: Bool?
    public let video: TelegramNativeVideo?
    public let views: Int64?
    public let reactions: [TelegramNativeReaction]?
    public let replyToMsgId: Int64?
    public let isAlbum: Bool?
    public let albumCount: Int?
    public let mediaList: [TelegramNativeMediaItem]?
    public let repliesResources: [TelegramNativeMessage]?
    public let isDuplicate: Bool?

    enum CodingKeys: String, CodingKey {
        case id, date, text, video, views, reactions
        case chatId = "chat_id"
        case isAd = "is_ad"
        case hasPhoto = "has_photo"
        case isVideo = "is_video"
        case replyToMsgId = "reply_to_msg_id"
        case isAlbum = "is_album"
        case albumCount = "album_count"
        case mediaList = "media_list"
        case repliesResources = "replies_resources"
        case isDuplicate = "is_duplicate"
    }
}
