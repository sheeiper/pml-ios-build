import ImageIO
import UIKit

@MainActor
final class ImageCache {
    static let shared = ImageCache()

    private let cache = NSCache<NSString, UIImage>()
    private let session: URLSession
    private var serverBaseURL: URL?
    private var serverAuthHeader: String?

    private init() {
        let configuration = URLSessionConfiguration.default
        configuration.requestCachePolicy = .returnCacheDataElseLoad
        configuration.urlCache = URLCache(memoryCapacity: 64 * 1024 * 1024, diskCapacity: 512 * 1024 * 1024)
        session = URLSession(configuration: configuration)
        cache.countLimit = 400
    }

    /// 服务器模式下，服务器同源图片（漫画页/封面/大图）需要 Basic 认证。
    /// 只对服务器 host 添加，避免把凭据发给 AList/115 CDN。
    func setServerAuth(baseURL: URL?, username: String, password: String) {
        serverBaseURL = baseURL
        if baseURL != nil, !username.isEmpty || !password.isEmpty {
            let raw = "\(username):\(password)"
            let encoded = Data(raw.utf8).base64EncodedString()
            serverAuthHeader = "Basic \(encoded)"
        } else {
            serverAuthHeader = nil
        }
    }

    private func isServerURL(_ url: URL) -> Bool {
        guard let serverBaseURL,
              let targetHost = url.host,
              let serverHost = serverBaseURL.host else { return false }
        let targetPort = url.port ?? (url.scheme == "https" ? 443 : 80)
        let serverPort = serverBaseURL.port ?? (serverBaseURL.scheme == "https" ? 443 : 80)
        return targetHost == serverHost && targetPort == serverPort
    }

    private func makeRequest(_ url: URL) -> URLRequest {
        var request = URLRequest(url: url)
        if let serverAuthHeader, isServerURL(url) {
            request.setValue(serverAuthHeader, forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private func key(_ url: URL, maxPixel: CGFloat?) -> NSString {
        "\(url.absoluteString)|\(maxPixel.map { Int($0.rounded()) } ?? 0)" as NSString
    }

    func cached(_ url: URL, maxPixel: CGFloat? = nil) -> UIImage? {
        cache.object(forKey: key(url, maxPixel: maxPixel))
    }

    func image(for url: URL, maxPixel: CGFloat? = nil) async -> UIImage? {
        if let cached = cached(url, maxPixel: maxPixel) { return cached }
        do {
            let (data, _) = try await session.data(for: makeRequest(url))
            guard let image = Self.decode(data, maxPixel: maxPixel) else { return nil }
            cache.setObject(image, forKey: key(url, maxPixel: maxPixel))
            return image
        } catch {
            return nil
        }
    }

    /// 先查内存缓存，未命中再走网络。`maxPixel` 用于浏览大图时降采样，避免 SwiftUI 缩放卡顿。
    func cachedOrLoad(_ url: URL, maxPixel: CGFloat? = nil) async -> UIImage? {
        if let cached = cached(url, maxPixel: maxPixel) { return cached }
        return await image(for: url, maxPixel: maxPixel)
    }

    private static func decode(_ data: Data, maxPixel: CGFloat?) -> UIImage? {
        guard let maxPixel, maxPixel > 0 else { return UIImage(data: data) }
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
            return UIImage(data: data)
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel,
        ]
        guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            return UIImage(data: data)
        }
        return UIImage(cgImage: cgImage)
    }
}

func pmlDownscaledJPEG(_ image: UIImage, maxPixel: CGFloat = 480, quality: CGFloat = 0.82) -> Data? {
    let longest = max(image.size.width, image.size.height)
    guard longest > 0 else { return image.jpegData(compressionQuality: quality) }
    let scale = min(1, maxPixel / longest)
    let target = CGSize(
        width: max(1, image.size.width * scale),
        height: max(1, image.size.height * scale)
    )
    let format = UIGraphicsImageRendererFormat.default()
    format.scale = 1
    let renderer = UIGraphicsImageRenderer(size: target, format: format)
    let resized = renderer.image { _ in
        image.draw(in: CGRect(origin: .zero, size: target))
    }
    return resized.jpegData(compressionQuality: quality)
}
