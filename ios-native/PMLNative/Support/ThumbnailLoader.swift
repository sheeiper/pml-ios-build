import SwiftUI
import UIKit

@MainActor
final class ThumbnailLoader: ObservableObject {
    @Published private(set) var images: [String: UIImage] = [:]
    private var inFlight = Set<String>()
    private var failed = Set<String>()

    func image(assetId: String, client: PMLAPIClient?) async -> UIImage? {
        if let cached = images[assetId] { return cached }
        guard let client, !inFlight.contains(assetId), !failed.contains(assetId) else { return nil }
        inFlight.insert(assetId)
        defer { inFlight.remove(assetId) }
        guard let thumbnail = try? await client.assetThumbnail(assetId: assetId),
              let data = Data(base64Encoded: thumbnail.dataBase64),
              let image = UIImage(data: data) else {
            failed.insert(assetId)
            return nil
        }
        images[assetId] = image
        return image
    }
}

func pmlIsCancellation(_ error: Error) -> Bool {
    if error is CancellationError { return true }
    let ns = error as NSError
    return ns.domain == NSURLErrorDomain && ns.code == NSURLErrorCancelled
}

func pmlFormatBytes(_ value: UInt64) -> String {
    guard value > 0 else { return "0 B" }
    let units = ["B", "KB", "MB", "GB", "TB"]
    var size = Double(value)
    var index = 0
    while size >= 1024 && index < units.count - 1 {
        size /= 1024
        index += 1
    }
    let text = index == 0 ? String(format: "%.0f", size) : String(format: "%.1f", size)
    return "\(text) \(units[index])"
}

func pmlFormatBytes(_ value: Int64) -> String {
    pmlFormatBytes(UInt64(max(0, value)))
}

func pmlFormatTime(_ seconds: Int64) -> String {
    guard seconds > 0 else { return "00:00" }
    let s = Int(seconds)
    let h = s / 3600
    let m = (s % 3600) / 60
    let sec = s % 60
    return h > 0
        ? String(format: "%d:%02d:%02d", h, m, sec)
        : String(format: "%02d:%02d", m, sec)
}
