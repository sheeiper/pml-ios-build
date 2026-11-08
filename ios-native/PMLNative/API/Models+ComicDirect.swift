import Foundation

struct ComicArchiveManifest: Decodable {
    let remotePath: String
    let pages: [ComicArchiveManifestPage]
}

struct ComicArchiveManifestPage: Decodable {
    let index: Int
    let name: String
    let compressionMethod: UInt16
    let flags: UInt16
    let compressedSize: UInt64
    let uncompressedSize: UInt64
    let localHeaderOffset: UInt64
}
