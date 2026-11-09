import SwiftUI

/// 复刻参考图的创作者卡片：背景大图 + 叠压头像 + 名称 + 真实作品数。
/// 不使用粉丝数、简介或外链等没有数据来源的字段。
struct AuthorShowcaseCard: View {
    let author: LibraryAuthor
    var bannerHeight: CGFloat = 108
    var isTopCreator: Bool = false

    @EnvironmentObject private var model: AppModel
    @ObservedObject private var thumbs = ThumbnailService.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ZStack(alignment: .bottomLeading) {
                banner
                    .frame(height: bannerHeight)
                    .frame(maxWidth: .infinity)
                    .clipped()
                    .overlay(alignment: .topTrailing) {
                        if isTopCreator {
                            Text("Top Creator")
                                .font(.system(size: 10, weight: .heavy))
                                .foregroundStyle(Color(red: 0.96, green: 0.82, blue: 0.54))
                                .padding(.horizontal, 9)
                                .padding(.vertical, 3)
                                .background(
                                    Capsule().fill(Color(red: 0.19, green: 0.14, blue: 0.05).opacity(0.75))
                                )
                                .overlay(
                                    Capsule().stroke(
                                        Color(red: 0.96, green: 0.82, blue: 0.54).opacity(0.55),
                                        lineWidth: 1
                                    )
                                )
                                .padding(10)
                        }
                    }
                    .clipShape(
                        UnevenRoundedRectangle(
                            topLeadingRadius: 18,
                            bottomLeadingRadius: 0,
                            bottomTrailingRadius: 0,
                            topTrailingRadius: 18,
                            style: .continuous
                        )
                    )

                avatar
                    .offset(x: 14, y: 30)
            }
            .padding(.bottom, 34)

            VStack(alignment: .leading, spacing: 6) {
                Text(author.name)
                    .font(.system(size: 16, weight: .bold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                HStack(spacing: 8) {
                    Text("@\(handle)")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Color(red: 0.65, green: 0.55, blue: 0.98))
                        .lineLimit(1)
                    Text("\(author.mediaCount) 部作品")
                        .font(.system(size: 11.5, weight: .semibold))
                        .foregroundStyle(.secondary)
                }

                Divider().padding(.top, 4)

                HStack(spacing: 6) {
                    Image(systemName: "photo.stack")
                    Text("\(author.collectionCount) 个合集")
                    Spacer(minLength: 4)
                    Text("作者档案").foregroundStyle(.tint)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.tint)
                }
                .font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(.secondary)
                .padding(.top, 4)
            }
            .padding(.horizontal, 14)
            .padding(.bottom, 12)
        }
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color(.secondarySystemBackground))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.06), lineWidth: 1)
        )
        .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .task(id: author.id) {
            if let avatarId = author.avatarAssetId ?? author.coverAssetId {
                await thumbs.image(assetId: avatarId, client: model.apiClient)
            }
            if let bannerId = author.bannerAssetId {
                await thumbs.image(assetId: bannerId, client: model.apiClient)
            }
        }
    }

    private var handle: String {
        let alias = author.aliases.first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty })
        let raw = alias ?? author.name
        return raw.hasPrefix("@") ? String(raw.dropFirst()) : raw
    }

    @ViewBuilder
    private var banner: some View {
        if let bannerId = author.bannerAssetId, let image = thumbs.images[bannerId] {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
        } else if let avatarId = author.avatarAssetId ?? author.coverAssetId,
                  let image = thumbs.images[avatarId] {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
                .blur(radius: 18)
                .overlay(Color.black.opacity(0.25))
        } else {
            LinearGradient(
                colors: [Color.accentColor.opacity(0.55), Color.accentColor.opacity(0.18)],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        }
    }

    private var avatar: some View {
        ZStack {
            Circle()
                .fill(
                    LinearGradient(
                        colors: [
                            Color(red: 0.98, green: 0.45, blue: 0.09),
                            Color(red: 0.93, green: 0.28, blue: 0.60),
                            Color(red: 0.55, green: 0.36, blue: 0.96),
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .frame(width: 66, height: 66)
            avatarContent
                .frame(width: 60, height: 60)
                .clipShape(Circle())
        }
        .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
    }

    @ViewBuilder
    private var avatarContent: some View {
        if let avatarId = author.avatarAssetId, let image = thumbs.images[avatarId] {
            Image(uiImage: image).resizable().scaledToFill()
        } else if let coverId = author.coverAssetId, let image = thumbs.images[coverId] {
            Image(uiImage: image).resizable().scaledToFill()
        } else {
            ZStack {
                LinearGradient(
                    colors: [Color.accentColor.opacity(0.85), Color.accentColor.opacity(0.45)],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
                Text(String(author.name.prefix(1)))
                    .font(.title2.weight(.bold))
                    .foregroundStyle(.white)
            }
        }
    }
}
