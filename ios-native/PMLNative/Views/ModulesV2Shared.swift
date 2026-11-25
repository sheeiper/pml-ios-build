import SwiftUI
import UIKit

/// 统一的 V2 视频播放 Sheet 载体。
struct V2VideoPlayPayload: Identifiable {
    let id = UUID()
    let url: URL
    let title: String
    let assetId: String
    let playlist: [VideoPlaylistItem]
    let playlistIndex: Int?
    let fallbackURL: URL?
    let sourceLabel: String?
    let remotePath: String?
}

/// 横向胶囊筛选条（工作室 / 频道 / 模特）。
struct V2FilterPill: View {
    let title: String
    let count: Int64?
    let isSelected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Text(title)
                    .font(.subheadline.weight(isSelected ? .semibold : .regular))
                    .lineLimit(1)
                if let count {
                    Text("\(count)")
                        .font(.caption2.monospacedDigit())
                        .opacity(isSelected ? 0.9 : 0.65)
                }
            }
            .padding(.horizontal, 13)
            .padding(.vertical, 7)
            .foregroundStyle(isSelected ? Color.white : Color.primary)
            .background(
                Capsule().fill(
                    isSelected
                        ? AnyShapeStyle(Color.accentColor)
                        : AnyShapeStyle(.ultraThinMaterial)
                )
            )
            .overlay(
                Capsule().strokeBorder(
                    isSelected ? Color.clear : Color.primary.opacity(0.08),
                    lineWidth: 0.8
                )
            )
        }
        .buttonStyle(.plain)
    }
}

/// 4 宫格 / 单图封面文件夹卡片（创作者子文件夹 & 专题合集通用）。
/// 当 `isSingleVideoWrapper == true` 时显示一键直放播放角标，提示点击直接播放。
struct V2FolderCardView: View {
    let title: String
    let subtitle: String?
    let previewAssetIds: [String]
    let coverAssetId: String?
    let itemCount: Int64
    let videoCount: Int64
    let imageCount: Int64
    let totalBytes: Int64
    let isSingleVideoWrapper: Bool
    let client: PMLAPIClient?

    private var gridIds: [String] {
        var ids = previewAssetIds.filter { !$0.isEmpty }
        if ids.isEmpty, let coverAssetId, !coverAssetId.isEmpty {
            ids = [coverAssetId]
        }
        return Array(ids.prefix(4))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack {
                coverArea
                    .frame(height: 118)
                    .frame(maxWidth: .infinity)
                    .clipped()

                if isSingleVideoWrapper {
                    Circle()
                        .fill(.ultraThinMaterial)
                        .frame(width: 42, height: 42)
                        .overlay {
                            Image(systemName: "play.fill")
                                .font(.system(size: 17, weight: .semibold))
                                .foregroundStyle(.white)
                                .offset(x: 1.5)
                        }
                        .shadow(color: .black.opacity(0.28), radius: 6, y: 2)
                }

                // 右下角角标
                VStack {
                    Spacer()
                    HStack {
                        if isSingleVideoWrapper {
                            Label("单视频直放", systemImage: "bolt.fill")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 7)
                                .padding(.vertical, 3)
                                .background(Color.accentColor.opacity(0.88), in: Capsule())
                        }
                        Spacer()
                        HStack(spacing: 3) {
                            Image(systemName: isSingleVideoWrapper ? "film.fill" : "folder.fill")
                                .font(.system(size: 9))
                            Text("\(itemCount)")
                                .font(.caption2.monospacedDigit().weight(.semibold))
                        }
                        .foregroundStyle(.white)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(Color.black.opacity(0.62), in: Capsule())
                    }
                    .padding(7)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)

                HStack(spacing: 6) {
                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .lineLimit(1)
                    } else {
                        Text(summaryText)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 2)
                    Text(PMLFormatters.bytes(totalBytes))
                        .monospacedDigit()
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 2)
        }
        .padding(8)
        .pmlGlassCard(cornerRadius: 16)
    }

    @ViewBuilder
    private var coverArea: some View {
        if gridIds.count >= 2 {
            let cells = gridIds.count >= 4 ? Array(gridIds.prefix(4)) : Array(gridIds.prefix(2))
            if cells.count == 4 {
                VStack(spacing: 1.5) {
                    HStack(spacing: 1.5) {
                        thumbCell(cells[0])
                        thumbCell(cells[1])
                    }
                    HStack(spacing: 1.5) {
                        thumbCell(cells[2])
                        thumbCell(cells[3])
                    }
                }
            } else {
                HStack(spacing: 1.5) {
                    thumbCell(cells[0])
                    thumbCell(cells[1])
                }
            }
        } else if let first = gridIds.first {
            thumbCell(first)
        } else {
            ZStack {
                Color.secondary.opacity(0.14)
                Image(systemName: isSingleVideoWrapper ? "play.rectangle.fill" : "folder.fill")
                    .font(.title2)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func thumbCell(_ assetId: String) -> some View {
        CachedImageView(
            url: client?.v2CoverURL(assetId: assetId),
            contentMode: .fill,
            minHeight: 56,
            maxPixel: 420
        )
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipped()
    }

    private var summaryText: String {
        var parts: [String] = []
        if videoCount > 0 { parts.append("\(videoCount) 视") }
        if imageCount > 0 { parts.append("\(imageCount) 图") }
        return parts.isEmpty ? "\(itemCount) 项" : parts.joined(separator: " · ")
    }
}

/// 单个文件卡片（视频或图片）。
struct V2AssetCardView: View {
    let asset: CreatorAssetV2
    let client: PMLAPIClient?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ZStack {
                CachedImageView(
                    url: client?.v2CoverURL(assetId: asset.effectiveThumbId, isArchive: asset.isComicArchive),
                    contentMode: .fill,
                    minHeight: 108,
                    maxPixel: 480
                )
                .frame(height: 108)
                .frame(maxWidth: .infinity)
                .clipped()

                if asset.isVideo {
                    Circle()
                        .fill(Color.black.opacity(0.45))
                        .frame(width: 34, height: 34)
                        .overlay {
                            Image(systemName: "play.fill")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(.white)
                                .offset(x: 1)
                        }
                }

                VStack {
                    Spacer()
                    HStack {
                        if let dur = PMLFormatters.duration(asset.durationSeconds) {
                            Text(dur)
                                .font(.caption2.monospacedDigit().weight(.semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(Color.black.opacity(0.68), in: Capsule())
                        }
                        Spacer()
                        Text(asset.isVideo ? "视频" : (asset.isComicArchive ? "归档" : "图片"))
                            .font(.caption2)
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Color.black.opacity(0.55), in: Capsule())
                    }
                    .padding(6)
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))

            Text(asset.fileName)
                .font(.caption.weight(.medium))
                .foregroundStyle(.primary)
                .lineLimit(2)

            Text(PMLFormatters.bytes(asset.sizeBytes))
                .font(.caption2.monospacedDigit())
                .foregroundStyle(.secondary)
        }
        .padding(7)
        .pmlGlassCard(cornerRadius: 14)
    }
}
