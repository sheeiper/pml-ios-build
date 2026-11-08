import Foundation

enum BrowseLayout: String, CaseIterable, Identifiable {
    case list
    case grid
    case wall

    var id: String { rawValue }

    var title: String {
        switch self {
        case .list: return "列表"
        case .grid: return "网格"
        case .wall: return "照片墙"
        }
    }

    var symbol: String {
        switch self {
        case .list: return "list.bullet"
        case .grid: return "square.grid.2x2"
        case .wall: return "rectangle.grid.2x2.fill"
        }
    }
}

enum BrowseSort: String, CaseIterable, Identifiable {
    case name
    case newest
    case size

    var id: String { rawValue }

    var title: String {
        switch self {
        case .name: return "名称"
        case .newest: return "最近修改"
        case .size: return "大小"
        }
    }
}

@MainActor
final class BrowsePreferences {
    static let shared = BrowsePreferences()

    private let defaults = UserDefaults.standard

    private func key(_ prefix: String, _ rootId: String, _ path: String) -> String {
        "pml.browse.\(prefix).\(rootId)|\(path)"
    }

    func layout(rootId: String, path: String) -> BrowseLayout {
        defaults.string(forKey: key("layout", rootId, path))
            .flatMap(BrowseLayout.init(rawValue:)) ?? .grid
    }

    func setLayout(_ value: BrowseLayout, rootId: String, path: String) {
        defaults.set(value.rawValue, forKey: key("layout", rootId, path))
    }

    func sort(rootId: String, path: String) -> BrowseSort {
        defaults.string(forKey: key("sort", rootId, path))
            .flatMap(BrowseSort.init(rawValue:)) ?? .name
    }

    func setSort(_ value: BrowseSort, rootId: String, path: String) {
        defaults.set(value.rawValue, forKey: key("sort", rootId, path))
    }
}
