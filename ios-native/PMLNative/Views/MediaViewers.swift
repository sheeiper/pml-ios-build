import SwiftUI

struct PreviewItem: Identifiable {
    let id = UUID()
    let url: URL
    let isVideo: Bool
    var startAt: Double = 0
    var assetId: String? = nil
    var title: String = ""
    var fallbackURL: URL? = nil
    var sourceLabel: String? = nil
    var remotePath: String? = nil
}

struct ImageViewerSheet: View {
    let url: URL
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image
                            .resizable()
                            .scaledToFit()
                    case .failure:
                        Label("图片加载失败", systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.white)
                    default:
                        ProgressView()
                            .tint(.white)
                    }
                }
            }
            .navigationTitle("预览")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("完成") { dismiss() }
                }
            }
        }
    }
}
