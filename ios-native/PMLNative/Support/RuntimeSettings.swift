import Foundation

enum PlaybackRouteMode: String, Codable, CaseIterable, Identifiable {
    case auto
    case direct
    case relay

    var id: String { rawValue }
    var title: String {
        switch self {
        case .auto: return "自动"
        case .direct: return "直连"
        case .relay: return "中转"
        }
    }
}

/// 纯服务器模式配置（无缝兼容旧版 UserDefaults 中的 serverURL / serverUser / serverPassword / alistURL）。
struct RuntimeSettings: Codable, Equatable {
    var serverURL: String = "http://103.119.1.41:18080"
    var serverUser: String = "admin"
    var serverPassword: String = ""
    var alistURL: String = "http://103.119.1.41:45112"
    var cd2Prefix: String = "/115open"
    var alistPrefix: String = "/115"
    var alistUser: String = ""
    var alistPassword: String = ""
    var playbackMode: PlaybackRouteMode? = .auto

    var routeMode: PlaybackRouteMode { playbackMode ?? .auto }

    var isConfigured: Bool {
        normalizedServerURL != nil && !serverPassword.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    static let storageKey = "pml.runtime.settings.v1"

    static func load() -> RuntimeSettings {
        guard let data = UserDefaults.standard.data(forKey: storageKey),
              let value = try? JSONDecoder().decode(RuntimeSettings.self, from: data) else {
            return RuntimeSettings()
        }
        return value
    }

    func save() {
        guard let data = try? JSONEncoder().encode(self) else { return }
        UserDefaults.standard.set(data, forKey: Self.storageKey)
    }

    var normalizedServerURL: URL? {
        var text = serverURL.trimmingCharacters(in: .whitespacesAndNewlines)
        while text.hasSuffix("/") { text.removeLast() }
        guard let url = URL(string: text), url.scheme == "http" || url.scheme == "https" else {
            return nil
        }
        return url
    }

    var normalizedAlistURL: URL? {
        var text = alistURL.trimmingCharacters(in: .whitespacesAndNewlines)
        while text.hasSuffix("/") { text.removeLast() }
        guard let url = URL(string: text), url.scheme == "http" || url.scheme == "https" else {
            return nil
        }
        return url
    }
}

struct PlaybackSource {
    let url: URL
    let fallbackURL: URL?
    let label: String?
}
