import Foundation

extension PMLAPIClient {
    /// ZIP 页清单：App 用它 + AList `/d` Range 做按页直连。
    /// ZIP64 / 加密包会抛错，调用方回退服务器漫画页路由。
    func comicArchiveManifest(assetId: String) async throws -> ComicArchiveManifest {
        try await command("get_comic_archive_manifest", arguments: ["assetId": assetId])
    }
}
