import Foundation

extension PMLAPIClient {
    /// Only JSON resolution uses server auth. AVPlayer receives a raw URL and no credentials.
    func directPlaybackURL(assetId: String?, remotePath: String?, alistURL: URL? = nil) async throws -> URL {
        struct Response: Decodable { let url: String }
        var arguments: [String: Any] = [:]
        if let assetId { arguments["assetId"] = assetId }
        if let remotePath { arguments["remotePath"] = remotePath }
        let response: Response = try await command("resolve_direct_playback", arguments: arguments)
        guard let url = URL(string: response.url),
              DirectPlaybackPolicy.allows(url, serverURL: configuration.baseURL, coreURL: nil, alistURL: alistURL) else {
            throw APIError.server("解析结果不是安全的媒体直链，已拒绝服务器中转")
        }
        return url
    }
}
