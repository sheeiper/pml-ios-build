import Foundation

struct DirectoryConfigPayload: Codable {
    var recentPath: String?
    var pendingPath: String?
    var vaultPath: String?
    var comicsPath: String?
}

struct SourceDirectoryListing: Decodable {
    let path: String
    let directories: [String]
}

struct AlistMappingCheck: Decodable {
    let ok: Bool
    let message: String
    let cd2Count: Int
    let alistCount: Int
    let matched: Int
    let missingInAlist: [String]
    let missingInCd2: [String]
}

extension PMLAPIClient {
    func directoryConfig() async throws -> DirectoryConfigPayload {
        try await command("get_directory_config")
    }

    func saveDirectoryConfig(_ config: DirectoryConfigPayload) async throws -> DirectoryConfigPayload {
        let arguments: [String: Any] = [
            "recentPath": config.recentPath ?? NSNull(),
            "pendingPath": config.pendingPath ?? NSNull(),
            "vaultPath": config.vaultPath ?? NSNull(),
            "comicsPath": config.comicsPath ?? NSNull(),
        ]
        return try await command("save_directory_config", arguments: arguments)
    }

    func listSourceDirectories(path: String) async throws -> SourceDirectoryListing {
        try await command("list_source_directories", arguments: ["path": path])
    }

    func listAlistDirectories(path: String) async throws -> SourceDirectoryListing {
        try await command("list_alist_directories", arguments: ["path": path])
    }

    func validateAlistMapping(cd2Path: String, alistPath: String) async throws -> AlistMappingCheck {
        try await command(
            "validate_alist_mapping",
            arguments: ["cd2Path": cd2Path, "alistPath": alistPath]
        )
    }
}
