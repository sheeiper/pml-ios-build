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

/// 缩略图服务：
/// 1. 内存缓存 + 正在进行的任务去重合并（避免 prefetch 与 cell.task 冲突返回 nil）；
/// 2. 所有 Base64 解码、ImageIO 降采样与位图预解压（preparingForDisplay）均在后台线程执行，零阻塞主线程；
/// 3. 对 ObservableObject 广播做节流合并（180ms），并为 DirectoryView 提供独立单 Cell 刷新的视图，彻底根治打开目录卡顿。
@MainActor
final class ThumbnailService: ObservableObject {
    static let shared = ThumbnailService()

    private(set) var images: [String: UIImage] = [:]
    private var accessOrder: [String] = []
    private var inFlight: [String: Task<UIImage?, Never>] = [:]
    private var failedAt: [String: Date] = [:]
    private let fetchGate = AsyncGate(limit: 8)
    private let generateGate = AsyncGate(limit: 2)
    private var notifyTask: Task<Void, Never>?

    private static let maxCachedImages = 450

    func cachedImage(assetId: String?) -> UIImage? {
        guard let assetId else { return nil }
        return images[assetId]
    }

    func image(assetId: String, client: PMLAPIClient?) async -> UIImage? {
        if let image = images[assetId] { return image }
        if let failed = failedAt[assetId], Date().timeIntervalSince(failed) < 30 { return nil }
        if let existing = inFlight[assetId] {
            return await existing.value
        }
        guard let client else { return nil }

        let task = Task<UIImage?, Never> { @MainActor in
            await self.fetchGate.acquire()
            let generated = await Self.loadOrGenerate(
                assetId: assetId,
                client: client,
                generateGate: self.generateGate
            )
            await self.fetchGate.release()
            return generated
        }
        inFlight[assetId] = task
        let result = await task.value
        inFlight.removeValue(forKey: assetId)

        if let result {
            storeImage(result, for: assetId)
            failedAt.removeValue(forKey: assetId)
            scheduleNotify()
            return result
        }
        failedAt[assetId] = Date()
        return nil
    }

    private func storeImage(_ image: UIImage, for assetId: String) {
        if images[assetId] == nil {
            accessOrder.append(assetId)
            if accessOrder.count > Self.maxCachedImages {
                let overflow = accessOrder.count - Self.maxCachedImages
                for oldKey in accessOrder.prefix(overflow) {
                    images.removeValue(forKey: oldKey)
                }
                accessOrder.removeFirst(overflow)
            }
        }
        images[assetId] = image
    }

    /// 节流通知旧的 @ObservedObject 订阅者，避免短时间几十张封面解码引发主线程重绘风暴。
    private func scheduleNotify() {
        guard notifyTask == nil else { return }
        notifyTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 180_000_000)
            self.notifyTask = nil
            self.objectWillChange.send()
        }
    }

    /// 目录加载后适量预取前 12 个封面。
    func prefetch(assetIds: [String], client: PMLAPIClient?) {
        guard let client else { return }
        var seen = Set<String>()
        let ids = assetIds.filter { seen.insert($0).inserted && images[$0] == nil }
        guard !ids.isEmpty else { return }
        Task { @MainActor in
            for id in ids.prefix(12) {
                if Task.isCancelled { break }
                if images[id] != nil || inFlight[id] != nil { continue }
                _ = await self.image(assetId: id, client: client)
            }
        }
    }

    func forget(assetId: String) {
        images.removeValue(forKey: assetId)
        failedAt.removeValue(forKey: assetId)
        scheduleNotify()
    }

    private static func loadOrGenerate(
        assetId: String,
        client: PMLAPIClient,
        generateGate: AsyncGate
    ) async -> UIImage? {
        guard let input = try? await client.thumbnailInput(assetId: assetId) else { return nil }

        // 1. 优先命中服务端已缓存 Base64 缩略图，并在后台线程解码 + 预解压位图
        if let cached = input.cached {
            let base64 = cached.dataBase64
            if let decoded = await Task.detached(priority: .utility, operation: { () -> UIImage? in
                guard let data = Data(base64Encoded: base64) else { return nil }
                return ImageCache.decodeOffMain(data, maxPixel: 360)
            }).value {
                return decoded
            }
        }

        // 2. 其次命中服务端缩略图 URL
        if let cachedUrl = input.cachedUrl,
           let url = try? client.resolve(cachedUrl),
           let image = await ImageCache.shared.image(for: url, maxPixel: 360) {
            return image
        }

        // 3. 未缓存时：仅对图片在独立低并发槽做降采样缩略图生成；避免在浏览文件夹时拉取远程大视频流抽帧卡死网络
        guard input.mediaType != "video",
              let source = input.source,
              let url = try? client.resolve(source) else {
            return nil
        }

        await generateGate.acquire()
        let image = await ImageCache.shared.image(for: url, maxPixel: 360)
        await generateGate.release()

        guard let image else { return nil }

        let version = input.version
        Task.detached(priority: .background) {
            guard let data = pmlDownscaledJPEG(image, maxPixel: 360, quality: 0.78), !data.isEmpty else { return }
            try? await client.saveAssetThumbnail(
                assetId: assetId,
                mimeType: "image/jpeg",
                dataBase64: data.base64EncodedString(),
                expectedVersion: version
            )
        }
        return image
    }
}

/// 独立单 Cell 缩略图组件：只刷新当前 Cell 自身状态，绝不触发父级列表或整个目录树重绘。
struct AssetThumbnailView<Placeholder: View>: View {
    let assetId: String?
    var contentMode: ContentMode = .fill
    @ViewBuilder let placeholder: () -> Placeholder

    @EnvironmentObject private var model: AppModel
    @State private var image: UIImage?
    @State private var loadedId: String?

    var body: some View {
        Group {
            if let current = resolvedImage {
                Image(uiImage: current)
                    .resizable()
                    .aspectRatio(contentMode: contentMode)
            } else {
                placeholder()
            }
        }
        .task(id: assetId) {
            guard let assetId, !assetId.isEmpty else {
                image = nil
                loadedId = nil
                return
            }
            if let cached = ThumbnailService.shared.cachedImage(assetId: assetId) {
                if loadedId != assetId || image == nil {
                    image = cached
                    loadedId = assetId
                }
                return
            }
            if loadedId != assetId {
                image = nil
                loadedId = assetId
            }
            if let fetched = await ThumbnailService.shared.image(assetId: assetId, client: model.apiClient),
               !Task.isCancelled {
                image = fetched
                loadedId = assetId
            }
        }
    }

    private var resolvedImage: UIImage? {
        guard let assetId else { return nil }
        if loadedId == assetId, let image {
            return image
        }
        return ThumbnailService.shared.cachedImage(assetId: assetId)
    }
}
