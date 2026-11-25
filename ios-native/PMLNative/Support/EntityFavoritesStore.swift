import Combine
import Foundation

enum EntityFavoriteType: String {
    case movie
    case performer
    case creator
}

struct EntityFavoriteIds: Decodable {
    let movies: [String]
    let performers: [String]
    let creators: [String]
}

/// 全局实体收藏状态（影片 / 演员 / 创作者），与服务端 `entity_favorites` 同步。
@MainActor
final class EntityFavoritesStore: ObservableObject {
    static let shared = EntityFavoritesStore()

    @Published private(set) var movies: Set<String> = []
    @Published private(set) var performers: Set<String> = []
    @Published private(set) var creators: Set<String> = []
    private var loaded = false

    func ensureLoaded(client: PMLAPIClient?, force: Bool = false) async {
        guard let client else { return }
        if loaded && !force { return }
        if let ids = try? await client.listEntityFavoriteIds() {
            movies = Set(ids.movies)
            performers = Set(ids.performers)
            creators = Set(ids.creators)
            loaded = true
        }
    }

    func isFavorited(_ type: EntityFavoriteType, id: String) -> Bool {
        switch type {
        case .movie: return movies.contains(id)
        case .performer: return performers.contains(id)
        case .creator: return creators.contains(id)
        }
    }

    func isMovieFav(_ id: String) -> Bool { isFavorited(.movie, id: id) }
    func isPerformerFav(_ id: String) -> Bool { isFavorited(.performer, id: id) }
    func isCreatorFav(_ id: String) -> Bool { isFavorited(.creator, id: id) }

    func toggle(_ type: EntityFavoriteType, id: String, client: PMLAPIClient?) async {
        guard let client, !id.isEmpty else { return }
        let wasFav = isFavorited(type, id: id)
        apply(type, id: id, favorited: !wasFav)
        do {
            let actual = try await client.toggleEntityFavorite(type: type, entityId: id)
            apply(type, id: id, favorited: actual)
        } catch {
            apply(type, id: id, favorited: wasFav)
        }
    }

    func toggle(client: PMLAPIClient?, type: EntityFavoriteType, id: String) async {
        await toggle(type, id: id, client: client)
    }

    private func apply(_ type: EntityFavoriteType, id: String, favorited: Bool) {
        switch type {
        case .movie:
            if favorited { movies.insert(id) } else { movies.remove(id) }
        case .performer:
            if favorited { performers.insert(id) } else { performers.remove(id) }
        case .creator:
            if favorited { creators.insert(id) } else { creators.remove(id) }
        }
    }
}
