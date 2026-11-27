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

    func pmlShimmer() -> some View {
        modifier(PMLShimmerModifier())
    }
}

private struct PMLShimmerModifier: ViewModifier {
    @State private var active = false

    func body(content: Content) -> some View {
        content
            .opacity(active ? 0.42 : 0.85)
            .animation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true), value: active)
            .onAppear { active = true }
    }
}

struct PMLSkeletonBlock: View {
    var cornerRadius: CGFloat = 10

    var body: some View {
        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
            .fill(Color.secondary.opacity(0.16))
            .pmlShimmer()
    }
}
