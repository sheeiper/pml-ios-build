import AVFoundation
import Combine
import SwiftUI
import UIKit

actor AsyncGate {
    private var available: Int
    private var waiters: [CheckedContinuation<Void, Never>] = []

    init(limit: Int) {
        available = limit
    }

    func acquire() async {
        if available > 0 {
            available -= 1
            return
        }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            waiters.append(continuation)
        }
    }

    func release() {
        if let next = waiters.first {
            waiters.removeFirst()
            next.resume()
        } else {
            available += 1
        }
    }
}

/// 缩略图服务：优先服务端缓存，其次按需生成（图片缩放 / 视频抽帧）并回写缓存。
@MainActor
final class ThumbnailService: ObservableObject {
    static let shared = ThumbnailService()

    @Published private(set) var images: [String: UIImage] = [:]
    private var inFlight = Set<String>()
    private var failedAt: [String: Date] = [:]
    private let gate = AsyncGate(limit: 4)

    func image(assetId: String, client: PMLAPIClient?) async -> UIImage? {
        if let image = images[assetId] { return image }
        if let failed = failedAt[assetId], Date().timeIntervalSince(failed) < 30 { return nil }
        guard let client, !inFlight.contains(assetId) else { return nil }
        inFlight.insert(assetId)
        await gate.acquire()

        let generated = await Self.generate(assetId: assetId, client: client)
        let cancelled = Task.isCancelled
        inFlight.remove(assetId)
        await gate.release()

        if let generated {
            images[assetId] = generated
            failedAt.removeValue(forKey: assetId)
            return generated
        }
        if !cancelled {
            failedAt[assetId] = Date()
        }
        return nil
    }

    /// 目录加载后并发预生成前 24 个封面；可见 cell 也会同时请求，共享 4 个并发槽。
    func prefetch(assetIds: [String], client: PMLAPIClient?) {
        guard let client else { return }
        var seen = Set<String>()
        let ids = assetIds.filter { seen.insert($0).inserted }
        Task { @MainActor in
            await withTaskGroup(of: Void.self) { group in
                for id in ids.prefix(24) {
                    if images[id] != nil { continue }
                    group.addTask { @MainActor in
                        _ = await self.image(assetId: id, client: client)
                    }
                }
            }
        }
    }

    func forget(assetId: String) {
        images.removeValue(forKey: assetId)
        failedAt.removeValue(forKey: assetId)
    }
    private static func generate(assetId: String, client: PMLAPIClient) async -> UIImage? {
        guard let input = try? await client.thumbnailInput(assetId: assetId) else { return nil }

        if let cached = input.cached,
           let data = Data(base64Encoded: cached.dataBase64),
           let image = UIImage(data: data) {
            return image
        }

        if let cachedUrl = input.cachedUrl,
           let url = try? client.resolve(cachedUrl),
           let image = await ImageCache.shared.image(for: url) {
            return image
        }

        guard let source = input.source, let url = try? client.resolve(source) else { return nil }

        let image: UIImage?
        if input.mediaType == "video" {
            image = await videoFrame(url: url)
        } else {
            image = await ImageCache.shared.image(for: url)
        }

        guard let image, let data = pmlDownscaledJPEG(image), !data.isEmpty else { return nil }

        try? await client.saveAssetThumbnail(
            assetId: assetId,
            mimeType: "image/jpeg",
            dataBase64: data.base64EncodedString(),
            expectedVersion: input.version
        )
        return image
    }

    private static func videoFrame(url: URL) async -> UIImage? {
        await Task.detached(priority: .utility) {
            let asset = AVURLAsset(url: url)
            let generator = AVAssetImageGenerator(asset: asset)
            generator.appliesPreferredTrackTransform = true
            generator.maximumSize = CGSize(width: 640, height: 640)
            let time = CMTime(seconds: 1.5, preferredTimescale: 600)
            guard let cgImage = try? generator.copyCGImage(at: time, actualTime: nil) else {
                return nil
            }
            return UIImage(cgImage: cgImage)
        }.value
    }
}
