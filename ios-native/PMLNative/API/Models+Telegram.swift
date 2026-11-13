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

public struct TelegramNativeChannelConfig: Codable, Hashable {
    public var showVideos: Bool
    public var showPhotos: Bool
    public var showLinks: Bool
    public var penetrateComments: Bool
    public var filterAds: Bool
    public var filterDuplicates: Bool

    public init(
        showVideos: Bool = true,
        showPhotos: Bool = true,
        showLinks: Bool = true,
        penetrateComments: Bool = true,
        filterAds: Bool = true,
        filterDuplicates: Bool = true
    ) {
        self.showVideos = showVideos
        self.showPhotos = showPhotos
        self.showLinks = showLinks
        self.penetrateComments = penetrateComments
        self.filterAds = filterAds
        self.filterDuplicates = filterDuplicates
    }

    enum CodingKeys: String, CodingKey {
        case showVideos = "show_videos"
        case showPhotos = "show_photos"
        case showLinks = "show_links"
        case penetrateComments = "penetrate_comments"
        case filterAds = "filter_ads"
        case filterDuplicates = "filter_duplicates"
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
    public var config: TelegramNativeChannelConfig?
    public let archived: Bool?
    public let pinned: Bool?
    public let folderIds: [Int]?
    public let username: String?
    public var unreadCount: Int?
    public let readInboxMaxId: Int64?
    public let lastMessage: String?
    public let date: String?

    enum CodingKeys: String, CodingKey {
        case id, title, username, archived, pinned, date, config
        case isSavedMessages = "is_saved_messages"
        case isChannel = "is_channel"
        case isGroup = "is_group"
        case isUser = "is_user"
        case isSubscribed = "is_subscribed"
        case folderIds = "folder_ids"
        case unreadCount = "unread_count"
        case readInboxMaxId = "read_inbox_max_id"
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

public struct TelegramNativeOfflineLink: Identifiable, Codable, Hashable {
    public var id: String { url }
    public let name: String
    public let url: String
    public let code: String?
    public let type: String
    public let copyText: String
    public let textWithoutLinks: String?
    public let source: String?

    enum CodingKeys: String, CodingKey {
        case name, url, code, type, source
        case copyText = "copy_text"
        case textWithoutLinks = "text_without_links"
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
    public let offlineLinks: [TelegramNativeOfflineLink]?
    public let views: Int64?
    public let reactions: [TelegramNativeReaction]?
    public let replyToMsgId: Int64?
    public let isAlbum: Bool?
    public let albumCount: Int?
    public let mediaList: [TelegramNativeMediaItem]?
    public let repliesResources: [TelegramNativeMessage]?
    public let repliesCount: Int?
    public let isDuplicate: Bool?
    public var isUnread: Bool?

    enum CodingKeys: String, CodingKey {
        case id, date, text, video, views, reactions
        case chatId = "chat_id"
        case isAd = "is_ad"
        case hasPhoto = "has_photo"
        case isVideo = "is_video"
        case offlineLinks = "offline_links"
        case replyToMsgId = "reply_to_msg_id"
        case isAlbum = "is_album"
        case albumCount = "album_count"
        case mediaList = "media_list"
        case repliesResources = "replies_resources"
        case repliesCount = "replies_count"
        case isDuplicate = "is_duplicate"
        case isUnread = "is_unread"
    }
}

public struct TelegramNativeComment: Identifiable, Codable, Hashable {
    public let id: Int64
    public let chatId: Int64?
    public let date: String?
    public let text: String?
    public let senderName: String?
    public let senderUsername: String?
    public let isVideo: Bool?
    public let hasPhoto: Bool?
    public let video: TelegramNativeVideo?
    public let offlineLinks: [TelegramNativeOfflineLink]?
    public let reactions: [TelegramNativeReaction]?

    enum CodingKeys: String, CodingKey {
        case id, date, text, video, reactions
        case chatId = "chat_id"
        case senderName = "sender_name"
        case senderUsername = "sender_username"
        case isVideo = "is_video"
        case hasPhoto = "has_photo"
        case offlineLinks = "offline_links"
    }
}

public struct TelegramNativeCommentsResponse: Codable {
    public let ok: Bool
    public let parentId: Int64?
    public let parentTitle: String?
    public let parentText: String?
    public let comments: [TelegramNativeComment]?
    public let offlineLinks: [TelegramNativeOfflineLink]?
    public let error: String?

    enum CodingKeys: String, CodingKey {
        case ok, error, comments
        case parentId = "parent_id"
        case parentTitle = "parent_title"
        case parentText = "parent_text"
        case offlineLinks = "offline_links"
    }
}
