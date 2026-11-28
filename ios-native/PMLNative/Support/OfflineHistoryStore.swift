import Foundation
import SwiftUI

@MainActor
final class OfflineHistoryStore: ObservableObject {
    static let shared = OfflineHistoryStore()

    @Published private(set) var hashes: Set<String> = []
    @Published private(set) var urls: Set<String> = []
    @Published private(set) var busyKey: String = ""

    private var loaded = false
    private var lastLoadedAt: Date = .distantPast
    private var inFlight: Task<Void, Never>?

    private init() {}

    static func cleanURL(_ input: String) -> String {
        input.trimmingCharacters(in: CharacterSet(charactersIn: " `'\"\n\r\t"))
    }

    static func extractBtih(_ input: String) -> String {
        let lower = cleanURL(input).lowercased()
        guard !lower.isEmpty else { return "" }
        if let range = lower.range(of: "btih:") {
            let tail = lower[range.upperBound...]
            let h = String(tail.prefix { $0.isLetter || $0.isNumber })
            return h.count >= 32 ? h : ""
        }
        if (lower.count == 32 || lower.count == 40), lower.allSatisfy({ $0.isLetter || $0.isNumber }) {
            return lower
        }
        return ""
    }

    static func canSubmitOffline(_ url: String, linkType: String? = nil) -> Bool {
        let clean = cleanURL(url).lowercased()
        guard !clean.isEmpty else { return false }
        if linkType == "magnet" || linkType == "ed2k" || linkType == "thunder" {
            return true
        }
        return clean.hasPrefix("magnet:?") || clean.hasPrefix("ed2k://") || clean.hasPrefix("thunder://")
    }

    func ensureLoaded(client: PMLAPIClient?, force: Bool = false) async {
        guard let client else { return }
        if !force && loaded && Date().timeIntervalSince(lastLoadedAt) < 60 {
            return
        }
        if let task = inFlight {
            await task.value
            return
        }
        let task = Task {
            defer { self.inFlight = nil }
            if let res = try? await client.listOfflinedSignatures() {
                self.hashes = Set(res.hashes.map { $0.lowercased() })
                self.urls = Set(res.urls.map { $0.lowercased() })
                self.loaded = true
                self.lastLoadedAt = Date()
            }
        }
        inFlight = task
        await task.value
    }

    func isOfflined(_ magnetOrHashOrURL: String) -> Bool {
        let clean = Self.cleanURL(magnetOrHashOrURL).lowercased()
        guard !clean.isEmpty else { return false }
        if urls.contains(clean) { return true }
        let btih = Self.extractBtih(clean)
        if !btih.isEmpty && hashes.contains(btih) { return true }
        return false
    }

    func markOfflined(magnet: String, infoHash: String? = nil) {
        let clean = Self.cleanURL(magnet).lowercased()
        if !clean.isEmpty {
            urls.insert(clean)
        }
        let btih = (infoHash?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()).flatMap { $0.isEmpty ? nil : $0 } ?? Self.extractBtih(clean)
        if !btih.isEmpty {
            hashes.insert(btih)
        }
    }

    func submit(
        client: PMLAPIClient,
        magnet: String,
        title: String? = nil,
        targetDir: String? = nil
    ) async throws -> OfflineDownloadTask {
        let clean = Self.cleanURL(magnet)
        let key = clean.lowercased()
        busyKey = key
        defer { busyKey = "" }
        let task = try await client.submitOfflineMagnet(magnet: clean, title: title, targetDir: targetDir)
        markOfflined(magnet: clean, infoHash: task.infoHash)
        return task
    }
}
