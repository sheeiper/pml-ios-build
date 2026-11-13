import Foundation
import UIKit

/// ZIP 漫画按页直连加速引擎：
/// 1. 单次合并 Range 请求（同时拉取 Local Header + 压缩数据），省去每页二次网络往返；
/// 2. 自动缓存 AList `/d` 302 重定向后的 CDN 直链（过期自动重取），翻页直达 CDN；
/// 3. 正在进行的页面请求去重合并（避免 prefetch 与视图加载重复下载同一页）；
/// 4. Deflate 解压与 ImageIO 位图预解码（preparingForDisplay）全在后台线程完成，零阻塞主线程滚动。
@MainActor
final class ComicDirectStore {
    static let shared = ComicDirectStore()

    private var manifests: [String: ComicArchiveManifest] = [:]
    private var unavailable: Set<String> = []
    private var images = NSCache<NSString, UIImage>()
    private var inFlight: [String: Task<UIImage?, Never>] = [:]
    private var redirectCache: [String: (url: URL, at: Date)] = [:]
    private let session: URLSession

    private init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 40
        configuration.httpMaximumConnectionsPerHost = 8
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: configuration)
        images.countLimit = 80
        images.totalCostLimit = 120 * 1024 * 1024
    }

    func remember(assetId: String, _ manifest: ComicArchiveManifest) {
        manifests[assetId] = manifest
        unavailable.remove(assetId)
    }

    func cachedImage(assetId: String, index: Int) -> UIImage? {
        images.object(forKey: key(assetId, index) as NSString)
    }

    func image(assetId: String, index: Int, model: AppModel) async -> UIImage? {
        let cacheKey = key(assetId, index)
        if let cached = images.object(forKey: cacheKey as NSString) { return cached }
        if unavailable.contains(assetId) { return nil }
        if let existing = inFlight[cacheKey] {
            return await existing.value
        }

        guard let client = model.apiClient else { return nil }
        let manifest: ComicArchiveManifest
        if let cached = manifests[assetId] {
            manifest = cached
        } else {
            do {
                manifest = try await client.comicArchiveManifest(assetId: assetId)
                manifests[assetId] = manifest
            } catch {
                unavailable.insert(assetId)
                return nil
            }
        }

        guard let page = manifest.pages.first(where: { $0.index == index }),
              (page.flags & 0x1) == 0,
              page.compressionMethod == 0 || page.compressionMethod == 8,
              page.compressedSize > 0,
              page.compressedSize <= UInt64(128 * 1024 * 1024),
              page.uncompressedSize > 0,
              page.uncompressedSize <= UInt64(128 * 1024 * 1024),
              let archiveURL = model.alistDirectURL(remotePath: manifest.remotePath) else {
            return nil
        }

        let task = Task<UIImage?, Never> { @MainActor in
            await self.fetchAndDecodePage(page: page, archiveURL: archiveURL)
        }
        inFlight[cacheKey] = task
        let loaded = await task.value
        inFlight.removeValue(forKey: cacheKey)

        if let loaded {
            let cost = Int(loaded.size.width * loaded.size.height * loaded.scale * loaded.scale * 4)
            images.setObject(loaded, forKey: cacheKey as NSString, cost: max(1, cost))
        }
        return loaded
    }

    func prefetch(assetId: String, index: Int, model: AppModel) {
        let cacheKey = key(assetId, index)
        guard images.object(forKey: cacheKey as NSString) == nil,
              inFlight[cacheKey] == nil else { return }
        Task { _ = await image(assetId: assetId, index: index, model: model) }
    }

    private func fetchAndDecodePage(page: ComicArchivePageEntry, archiveURL: URL) async -> UIImage? {
        let compLen = Int(page.compressedSize)
        let uncompLen = Int(page.uncompressedSize)
        let method = page.compressionMethod
        // 预留 30 字节 Local Header + 256 字节文件名与扩展字段，绝大多数情况 1 次 Range 请求即可拿全
        let speculativeLen = 30 + 256 + compLen

        do {
            let chunk = try await rangeWithRedirectCache(
                archiveURL: archiveURL,
                start: page.localHeaderOffset,
                minLength: 30,
                requestLength: speculativeLen
            )
            let nameLen = Int(Self.readUInt16(chunk, 26))
            let extraLen = Int(Self.readUInt16(chunk, 28))
            let headerSkip = 30 + nameLen + extraLen

            let compressed: Data
            if chunk.count >= headerSkip + compLen {
                let startIdx = chunk.startIndex + headerSkip
                compressed = Data(chunk[startIdx ..< (startIdx + compLen)])
            } else {
                let dataOffset = page.localHeaderOffset + UInt64(headerSkip)
                compressed = try await rangeWithRedirectCache(
                    archiveURL: archiveURL,
                    start: dataOffset,
                    minLength: compLen,
                    requestLength: compLen
                )
            }

            return await Task.detached(priority: .userInitiated) {
                let payload: Data
                if method == 0 {
                    payload = compressed
                } else {
                    guard let inflated = try? Self.inflateRaw(compressed, expected: uncompLen) else {
                        return nil
                    }
                    payload = inflated
                }
                return ImageCache.decodeOffMain(payload, maxPixel: 1800)
            }.value
        } catch {
            return nil
        }
    }

    private func rangeWithRedirectCache(
        archiveURL: URL,
        start: UInt64,
        minLength: Int,
        requestLength: Int
    ) async throws -> Data {
        let keyStr = archiveURL.absoluteString
        if let cached = redirectCache[keyStr], Date().timeIntervalSince(cached.at) < 480 {
            if let (data, _) = try? await performRange(url: cached.url, start: start, minLength: minLength, requestLength: requestLength) {
                return data
            }
            redirectCache.removeValue(forKey: keyStr)
        }
        let (data, finalURL) = try await performRange(url: archiveURL, start: start, minLength: minLength, requestLength: requestLength)
        if let finalURL, finalURL != archiveURL {
            redirectCache[keyStr] = (finalURL, Date())
        }
        return data
    }

    private func performRange(
        url: URL,
        start: UInt64,
        minLength: Int,
        requestLength: Int
    ) async throws -> (Data, URL?) {
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("bytes=\(start)-\(start + UInt64(max(1, requestLength)) - 1)", forHTTPHeaderField: "Range")
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw ComicDirectError.badResponse }
        guard http.statusCode == 206 || http.statusCode == 200 else { throw ComicDirectError.badResponse }
        guard data.count >= minLength else { throw ComicDirectError.shortData }
        return (data, http.url)
    }

    nonisolated private static func inflateRaw(_ data: Data, expected: Int) throws -> Data {
        var output = [UInt8](repeating: 0, count: expected)
        let written: Int = data.withUnsafeBytes { source in
            output.withUnsafeMutableBytes { destination in
                let src = source.bindMemory(to: UInt8.self).baseAddress
                let dst = destination.bindMemory(to: UInt8.self).baseAddress
                return Int(pml_inflate_raw(src, data.count, dst, expected))
            }
        }
        guard written > 0 else { throw ComicDirectError.inflateFailed }
        return Data(output.prefix(written))
    }

    private func key(_ assetId: String, _ index: Int) -> String {
        "\(assetId)#\(index)"
    }

    nonisolated private static func readUInt16(_ data: Data, _ offset: Int) -> UInt16 {
        guard data.count >= offset + 2 else { return 0 }
        let start = data.startIndex + offset
        return UInt16(data[start]) | (UInt16(data[start + 1]) << 8)
    }
}

private enum ComicDirectError: Error {
    case badResponse
    case shortData
    case inflateFailed
}
