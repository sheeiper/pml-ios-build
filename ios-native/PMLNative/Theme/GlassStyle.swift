import SwiftUI

extension View {
    /// 最低支持 iOS 26，直接使用系统 Liquid Glass 材质。
    @ViewBuilder
    func pmlGlassCard(cornerRadius: CGFloat = 24) -> some View {
        self.glassEffect(
            .regular,
            in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        )
    }
}
