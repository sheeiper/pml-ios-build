import UIKit

/// 把相册图片压成 JPEG 并 Base64，用于封面上传（后端限制 8 MB）。
func pmlJPEGBase64(_ data: Data, maxDimension: CGFloat = 1600, quality: CGFloat = 0.85) -> String? {
    guard let image = UIImage(data: data) else { return nil }
    let width = image.size.width
    let height = image.size.height
    guard width > 0, height > 0 else { return nil }
    let scale = min(1, maxDimension / max(width, height))
    let target = CGSize(width: width * scale, height: height * scale)

    let format = UIGraphicsImageRendererFormat.default()
    format.scale = 1
    let renderer = UIGraphicsImageRenderer(size: target, format: format)
    let resized = renderer.image { _ in
        image.draw(in: CGRect(origin: .zero, size: target))
    }
    return resized.jpegData(compressionQuality: quality)?.base64EncodedString()
}
