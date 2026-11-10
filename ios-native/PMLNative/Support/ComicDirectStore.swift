import Foundation
import UIKit

/// ZIP 漫画按页直连：服务器只给清单，页面压缩字节由手机向 AList `/d` 发 Range 请求并本地解压。
/// ZIP64/加密/失败时返回 nil，由 ComicDirectPageView 回退服务器 `/api/comic-pages`。
@MainActor
final class ComicDirectStore {
    static let shared = ComicDirectStore()

    private var manifests: [String: ComicArchiveManifest] = [:]
    private var unavailable: Set<String> = []
    private var images = NSCache<NSString, UIImage>()
    private let session: URLSession

    private init() {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 40
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: configuration)
        images.countLimit = 80
    }

    func remember(assetId: String, _ manifest: ComicArchiveManifest) {
        manifests[assetId] = manifest
        unavailable.remove(assetId)
    }

    func cachedImage(assetId: String, index: Int) -> UIImage? {
        images.object(forKey: key(assetId, index))
    }

    func image(assetId: String, index: Int, model: AppModel) async -> UIImage? {
        let cacheKey = key(assetId, index)
        if let cached = images.object(forKey: cacheKey) { return cached }
        if unavailable.contains(assetId) { return nil }

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

        do {
            let header = try await range(archiveURL, start: page.localHeaderOffset, length: 30)
            guard header.count >= 30 else { return nil }
            let nameLen = Int(readUInt16(header, 26))
            let extraLen = Int(readUInt16(header, 28))
            let dataOffset = page.localHeaderOffset + 30 + UInt64(nameLen) + UInt64(extraLen)
            let compressed = try await range(archiveURL, start: dataOffset, length: Int(page.compressedSize))

            let payload: Data
            if page.compressionMethod == 0 {
                payload = compressed
            } else {
                payload = try inflateRaw(compressed, expected: Int(page.uncompressedSize))
            }
            guard let image = UIImage(data: payload) else { return nil }
            images.setObject(image, forKey: cacheKey)
            return image
        } catch {
            return nil
        }
    }

    func prefetch(assetId: String, index: Int, model: AppModel) {
        Task { _ = await image(assetId: assetId, index: index, model: model) }
    }

    private func range(_ url: URL, start: UInt64, length: Int) async throws -> Data {
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("bytes=\(start)-\(start + UInt64(length) - 1)", forHTTPHeaderField: "Range")
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw ComicDirectError.badResponse }
        guard http.statusCode == 206 || http.statusCode == 200 else { throw ComicDirectError.badResponse }
        guard data.count >= length else { throw ComicDirectError.shortData }
        return Data(data.prefix(length))
    }

    private func inflateRaw(_ data: Data, expected: Int) throws -> Data {
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

    private func key(_ assetId: String, _ index: Int) -> NSString {
        "\(assetId)#\(index)" as NSString
    }

    private func readUInt16(_ data: Data, _ offset: Int) -> UInt16 {
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
