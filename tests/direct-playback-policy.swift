import Foundation

@main
struct DirectPlaybackPolicyTests {
    static func main() {
        let server = URL(string: "https://pml.example:18080")!
        let core = URL(string: "http://127.0.0.1:51999")!
        var checked = 0
        func check(_ raw: String, _ expected: Bool) {
            let actual = DirectPlaybackPolicy.allows(URL(string: raw)!, serverURL: server, coreURL: core)
            precondition(actual == expected, "Unexpected direct playback decision: \(raw)")
            checked += 1
        }
        check("https://cdn.example/video.mp4?sign=short-lived", true)
        check("http://192.168.1.2:5244/video.mp4", true)
        check("https://pml.example:5244/video.mp4", true) // same host, other port
        check("https://pml.example:18080/media/asset.mp4", false)
        check("https://pml.example:18080/custom-stream", false)
        check("https://pml.example:18080/%6dedia/asset.mp4", false)
        check("http://elsewhere.example/media/asset.mp4", false)
        check("http://127.0.0.1:51999/media/asset.mp4", false)
        check("http://127.0.0.2:1234/stream", false)
        check("http://localhost:1234/stream", false)
        check("http://[::1]:1234/stream", false)
        check("https://user:password@cdn.example/video.mp4", false)
        check("file:///tmp/video.mp4", false)
        check("ftp://cdn.example/video.mp4", false)
        check("https://alist.example:5244/d/115/movie.mp4", true)
        check("https://cdn.example/dav/video.mp4", false)
        check("https://cdn.example/p/video.mp4", false)
        check("https://cdn.example/api/telegram_user_stream", false)
        precondition(!DirectPlaybackPolicy.allows(URL(string:"https://PML.EXAMPLE:443/other")!, serverURL: URL(string:"https://pml.example")!, coreURL: nil))
        let sameOriginAlist = URL(string: "https://pml.example:18080/alist")!
        precondition(DirectPlaybackPolicy.allows(URL(string: "https://pml.example:18080/alist/d/video.mp4")!, serverURL: server, coreURL: nil, alistURL: sameOriginAlist))
        precondition(!DirectPlaybackPolicy.allows(URL(string: "https://pml.example:18080/alist/media/video.mp4")!, serverURL: server, coreURL: nil, alistURL: sameOriginAlist))
        precondition(DirectPlaybackPolicy.failureMessage.contains("不会切换服务器中转"))
        print("DirectPlaybackPolicy: \(checked + 2) assertions passed")
    }
}
