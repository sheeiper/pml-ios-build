import Foundation

struct SiblingVideo: Decodable {
    let id: String
    let title: String
}

struct SubtitleTrack: Decodable, Identifiable {
    let name: String
    let path: String
    let format: String

    var id: String { path }
}

extension PMLAPIClient {
    func listSiblingVideos(assetId: String) async throws -> [SiblingVideo] {
        try await command("list_sibling_videos", arguments: ["assetId": assetId])
    }

    func findAssetSubtitles(assetId: String) async throws -> [SubtitleTrack] {
        try await command("find_asset_subtitles", arguments: ["assetId": assetId])
    }

    /// 字幕文件走媒体代理，取回文本并做编码兜底（UTF-8 / UTF-16 / GB18030）。
    func subtitleText(remotePath: String) async throws -> String {
        let raw: String = try await command("open_raw_media_source", arguments: ["remotePath": remotePath])
        let url = try resolve(raw)
        let (data, _) = try await APISession.shared.session.data(from: url)
        if let text = String(data: data, encoding: .utf8) { return text }
        if let text = String(data: data, encoding: .utf16) { return text }
        let gb18030 = String.Encoding(rawValue: 0x80000632)
        if let text = String(data: data, encoding: gb18030) { return text }
        return String(decoding: data, as: UTF8.self)
    }
}