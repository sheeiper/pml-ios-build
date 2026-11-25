import ImageIO
import UIKit

@MainActor
final class ImageCache {
    static let shared = ImageCache()

    private let cache = NSCache<NSString, UIImage>()
    private let session: URLSession
    private var inFlight: [String: Task<UIImage?, Never>] = [:]
    private var serverBaseURL: URL?
    private var serverAuthHeader: String?

    private init() {
        let configuration = URLSessionConfiguration.default
        configuration.requestCachePolicy = .returnCacheDataElseLoad
        configuration.httpMaximumConnectionsPerHost = 6
        configuration.timeoutIntervalForRequest = 35
        configuration.urlCache = URLCache(memoryCapacity: 64 * 1024 * 1024, diskCapacity: 512 * 1024 * 1024)
        session = URLSession(configuration: configuration)
        cache.countLimit = 400
        cache.totalCostLimit = 160 * 1024 * 1024
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

    private func keyString(_ url: URL, maxPixel: CGFloat?) -> String {
        "\(url.absoluteString)|\(maxPixel.map { Int($0.rounded()) } ?? 0)"
    }

    func cached(_ url: URL, maxPixel: CGFloat? = nil) -> UIImage? {
        cache.object(forKey: keyString(url, maxPixel: maxPixel) as NSString)
    }

    func image(for url: URL, maxPixel: CGFloat? = nil) async -> UIImage? {
        let k = keyString(url, maxPixel: maxPixel)
        if let hit = cache.object(forKey: k as NSString) { return hit }
        if let existing = inFlight[k] {
            return await existing.value
        }

        let request = makeRequest(url)
        let session = self.session
        let isServerThumb = isServerURL(url) && (url.path.hasPrefix("/thumbnail/") || url.path.hasPrefix("/api/comic-pages/"))
        let task = Task<UIImage?, Never> {
            let maxAttempts = isServerThumb ? 3 : 2
            for attempt in 0..<maxAttempts {
                if Task.isCancelled { return nil }
                do {
                    if isServerThumb {
                        await ImageFetchGate.shared.acquire()
                    }
                    defer {
                        if isServerThumb {
                            Task { await ImageFetchGate.shared.release() }
                        }
                    }
                    let (data, response) = try await session.data(for: request)
                    if let http = response as? HTTPURLResponse {
                        if (200...299).contains(http.statusCode) {
                            let decoded = await Task.detached(priority: .userInitiated) {
                                Self.decodeOffMain(data, maxPixel: maxPixel)
                            }.value
                            if let decoded {
                                return decoded
                            }
                            return nil
                        }
                        let retryable = http.statusCode == 503 || http.statusCode == 504 || http.statusCode == 429 || http.statusCode == 502
                        if !retryable || attempt == maxAttempts - 1 {
                            return nil
                        }
                    }
                } catch {
                    if Task.isCancelled || attempt == maxAttempts - 1 {
                        return nil
                    }
                }
                let backoffNs = UInt64((attempt == 0 ? 1_500 : (attempt == 1 ? 3_500 : 6_000)) * 1_000_000)
                try? await Task.sleep(nanoseconds: backoffNs)
            }
            return nil
        }
        inFlight[k] = task
        let loaded = await task.value
        inFlight.removeValue(forKey: k)

        if let loaded {
            let cost = Int(loaded.size.width * loaded.size.height * loaded.scale * loaded.scale * 4)
            cache.setObject(loaded, forKey: k as NSString, cost: max(1, cost))
        }
        return loaded
    }

    /// 先查内存缓存，未命中再走网络。`maxPixel` 用于浏览大图时降采样，避免 SwiftUI 缩放卡顿。
    func cachedOrLoad(_ url: URL, maxPixel: CGFloat? = nil) async -> UIImage? {
        if let cached = cached(url, maxPixel: maxPixel) { return cached }
        return await image(for: url, maxPixel: maxPixel)
    }

    nonisolated static func decodeOffMain(_ data: Data, maxPixel: CGFloat?) -> UIImage? {
        guard !data.isEmpty else { return nil }
        if let maxPixel, maxPixel > 0,
           let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) {
            let options: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceShouldCacheImmediately: true,
                kCGImageSourceThumbnailMaxPixelSize: maxPixel,
            ]
            if let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) {
                let img = UIImage(cgImage: cgImage)
                return img.preparingForDisplay() ?? img
            }
        }
        guard let img = UIImage(data: data) else { return nil }
        return img.preparingForDisplay() ?? img
    }
}

nonisolated func pmlDownscaledJPEG(_ image: UIImage, maxPixel: CGFloat = 480, quality: CGFloat = 0.82) -> Data? {
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

/// 限制服务器动态缩略图/压缩包封面的并发拉取数量，避免 4 宫格卡片瞬间打爆服务端抽帧队列。
private actor ImageFetchGate {
    static let shared = ImageFetchGate(limit: 4)
    private let limit: Int
    private var active = 0
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(limit: Int) {
        self.limit = limit
    }

    func acquire() async {
        if active < limit {
            active += 1
            return
        }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func release() {
        if !waiters.isEmpty {
            let next = waiters.removeFirst()
            next.resume()
        } else {
            active = max(0, active - 1)
        }
    }
}
