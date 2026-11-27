import SwiftUI
import Metal
import UIKit

struct ComicArchiveTarget: Identifiable {
    let id: String
    let title: String
}

enum ReaderPageSource: Identifiable, Hashable {
    case url(URL)
    case comicArchivePage(assetId: String, pageIndex: Int, fallbackURL: URL)
    case remoteAsset(assetId: String, remotePath: String, fallbackURL: URL)

    var id: String {
        switch self {
        case .url(let u):
            return "url:\(u.absoluteString)"
        case .comicArchivePage(let aid, let idx, _):
            return "arc:\(aid)#\(idx)"
        case .remoteAsset(let aid, let path, _):
            return "rem:\(aid)|\(path)"
        }
    }

    var cacheKey: String { id }
}

// MARK: - Metal GPU Image Enhancer (Anime4K + FidelityFX CAS) & Smart Auto-Crop

enum ReaderEnhanceMode: String, CaseIterable, Identifiable {
    case off = "off"
    case anime = "anime"
    case photo = "photo"
    var id: String { rawValue }
    var label: String {
        switch self {
        case .off: return "关闭"
        case .anime: return "漫画增强"
        case .photo: return "写真增强"
        }
    }
    var shortLabel: String {
        switch self {
        case .off: return "原图"
        case .anime: return "漫画增强"
        case .photo: return "写真增强"
        }
    }
}

enum ReaderEnhanceStrength: String, CaseIterable, Identifiable {
    case light = "light"
    case standard = "standard"
    case strong = "strong"
    var id: String { rawValue }
    var label: String {
        switch self {
        case .light: return "轻"
        case .standard: return "标准"
        case .strong: return "强"
        }
    }
    var factor: Float {
        switch self {
        case .light: return 0.65
        case .standard: return 1.0
        case .strong: return 1.45
        }
    }
}

enum ReaderLayoutMode: String, CaseIterable, Identifiable {
    case strip = "strip"
    case single = "single"
    case manga = "manga"
    case double = "double"
    var id: String { rawValue }
    var label: String {
        switch self {
        case .strip: return "条漫"
        case .single: return "单页"
        case .manga: return "日漫 (右→左)"
        case .double: return "双页"
        }
    }
    var shortLabel: String {
        switch self {
        case .strip: return "条漫"
        case .single: return "单页"
        case .manga: return "日漫"
        case .double: return "双页"
        }
    }
}

enum ReaderKind: String {
    case comic
    case gallery
}

final class MetalImageEnhancer {
    static let shared = MetalImageEnhancer()

    private let device: MTLDevice?
    private let queue: MTLCommandQueue?
    private let animePipeline: MTLComputePipelineState?
    private let photoPipeline: MTLComputePipelineState?
    private let cache = NSCache<NSString, UIImage>()

    private static let shaderSource = """
    #include <metal_stdlib>
    using namespace metal;

    inline float luma(float3 col) {
        return dot(col, float3(0.299, 0.587, 0.114));
    }

    inline float skin_likelihood(float3 col) {
        float maxC = max(col.r, max(col.g, col.b));
        float minC = min(col.r, min(col.g, col.b));
        float rgDiff = col.r - col.g;
        float warm = smoothstep(0.02, 0.14, rgDiff) * smoothstep(0.01, 0.12, col.g - col.b);
        float satOk = smoothstep(0.04, 0.22, maxC - minC) * smoothstep(0.75, 0.35, maxC - minC);
        float lumOk = smoothstep(0.18, 0.42, col.r) * smoothstep(0.98, 0.72, col.r);
        return clamp(warm * satOk * lumOk, 0.0, 1.0);
    }

    kernel void enhance_anime(
        texture2d<float, access::sample> inTex [[texture(0)]],
        texture2d<float, access::write> outTex [[texture(1)]],
        constant float &strength [[buffer(0)]],
        uint2 gid [[thread_position_in_grid]]
    ) {
        uint outW = outTex.get_width();
        uint outH = outTex.get_height();
        if (gid.x >= outW || gid.y >= outH) return;

        constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::linear);
        float2 uv = (float2(gid) + 0.5) / float2(outW, outH);
        float2 texel = 1.0 / float2(inTex.get_width(), inTex.get_height());

        float3 p00 = inTex.sample(s, uv + float2(-texel.x, -texel.y)).rgb;
        float3 p10 = inTex.sample(s, uv + float2( 0.0,     -texel.y)).rgb;
        float3 p20 = inTex.sample(s, uv + float2( texel.x, -texel.y)).rgb;
        float3 p01 = inTex.sample(s, uv + float2(-texel.x,  0.0)).rgb;
        float3 p11 = inTex.sample(s, uv).rgb;
        float3 p21 = inTex.sample(s, uv + float2( texel.x,  0.0)).rgb;
        float3 p02 = inTex.sample(s, uv + float2(-texel.x,  texel.y)).rgb;
        float3 p12 = inTex.sample(s, uv + float2( 0.0,      texel.y)).rgb;
        float3 p22 = inTex.sample(s, uv + float2( texel.x,  texel.y)).rgb;

        float3 q01 = inTex.sample(s, uv + float2(-2.0 * texel.x, 0.0)).rgb;
        float3 q21 = inTex.sample(s, uv + float2( 2.0 * texel.x, 0.0)).rgb;
        float3 q10 = inTex.sample(s, uv + float2(0.0, -2.0 * texel.y)).rgb;
        float3 q12 = inTex.sample(s, uv + float2(0.0,  2.0 * texel.y)).rgb;

        // 1. Directional gradient (Sobel) for manga ink line detection
        float gx = (luma(p20) + 2.0 * luma(p21) + luma(p22)) - (luma(p00) + 2.0 * luma(p01) + luma(p02));
        float gy = (luma(p02) + 2.0 * luma(p12) + luma(p22)) - (luma(p00) + 2.0 * luma(p10) + luma(p20));
        float grad = clamp(sqrt(gx * gx + gy * gy), 0.0, 1.0);

        // 2. Edge-preserving flat-tone denoise (removes JPEG mosquito noise without softening lines)
        float3 avgFlat = (p11 * 4.0 + (p10 + p12 + p01 + p21) * 2.0 + (p00 + p20 + p02 + p22)) / 16.0;
        float flatW = smoothstep(0.18, 0.02, grad) * clamp(0.45 * strength, 0.15, 0.65);
        float3 denoised = mix(p11, avgFlat, flatW);

        // 3. Scan auto-levels: deepen washed-out ink blacks & whiten dingy paper highlights
        float l0 = luma(denoised);
        float shadowPush = smoothstep(0.28, 0.04, l0) * 0.16 * strength;
        float paperLift = smoothstep(0.76, 0.96, l0) * 0.10 * strength;
        float3 leveled = clamp(denoised * (1.0 - shadowPush) + float3(paperLift * (1.0 - l0 * 0.15)), 0.0, 1.0);
        leveled = clamp((leveled - 0.03 * strength) / max(0.78, 1.0 - 0.06 * strength), 0.0, 1.0);

        // 4. Multi-scale unsharp mask (1px fine + 2px edge)
        float3 blur1 = (p10 + p12 + p01 + p21) * 0.25;
        float3 blur2 = (q10 + q12 + q01 + q21) * 0.25;
        float3 detail1 = denoised - blur1;
        float3 detail2 = denoised - blur2;
        float sharp1 = mix(0.65, 1.95, smoothstep(0.03, 0.30, grad)) * strength;
        float sharp2 = mix(0.20, 0.85, smoothstep(0.04, 0.35, grad)) * strength;
        float3 sharpened = leveled + detail1 * sharp1 + detail2 * sharp2;

        // 5. Anime4K dark-line tightening for crisp manga ink strokes
        float3 minNb = min(min(min(p10, p12), min(p01, p21)), p11);
        float3 maxNb = max(max(max(p10, p12), max(p01, p21)), p11);
        float inkMask = smoothstep(0.62, 0.14, luma(p11)) * smoothstep(0.05, 0.28, grad);
        sharpened = mix(sharpened, min(sharpened, minNb * 0.88), clamp(inkMask * 0.55 * strength, 0.0, 0.78));

        // 6. Smart vibrance for full-color manga (neutral on B&W pages)
        float maxC = max(sharpened.r, max(sharpened.g, sharpened.b));
        float minC = min(sharpened.r, min(sharpened.g, sharpened.b));
        float chroma = maxC - minC;
        float colorMask = smoothstep(0.04, 0.18, chroma);
        float lSharp = luma(sharpened);
        float3 vib = mix(float3(lSharp), sharpened, 1.0 + 0.16 * strength * colorMask);

        // 7. Adaptive halo clamp (wider allowance for real contrast gain without ringing)
        float3 finalCol = clamp(vib, minNb - float3(0.22), maxNb + float3(0.22));
        outTex.write(float4(clamp(finalCol, 0.0, 1.0), 1.0), gid);
    }

    kernel void enhance_photo(
        texture2d<float, access::sample> inTex [[texture(0)]],
        texture2d<float, access::write> outTex [[texture(1)]],
        constant float &strength [[buffer(0)]],
        uint2 gid [[thread_position_in_grid]]
    ) {
        uint outW = outTex.get_width();
        uint outH = outTex.get_height();
        if (gid.x >= outW || gid.y >= outH) return;

        constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::linear);
        float2 uv = (float2(gid) + 0.5) / float2(outW, outH);
        float2 texel = 1.0 / float2(inTex.get_width(), inTex.get_height());

        float3 p00 = inTex.sample(s, uv + float2(-texel.x, -texel.y)).rgb;
        float3 p10 = inTex.sample(s, uv + float2( 0.0,     -texel.y)).rgb;
        float3 p20 = inTex.sample(s, uv + float2( texel.x, -texel.y)).rgb;
        float3 p01 = inTex.sample(s, uv + float2(-texel.x,  0.0)).rgb;
        float3 p11 = inTex.sample(s, uv).rgb;
        float3 p21 = inTex.sample(s, uv + float2( texel.x,  0.0)).rgb;
        float3 p02 = inTex.sample(s, uv + float2(-texel.x,  texel.y)).rgb;
        float3 p12 = inTex.sample(s, uv + float2( 0.0,      texel.y)).rgb;
        float3 p22 = inTex.sample(s, uv + float2( texel.x,  texel.y)).rgb;

        float3 q01 = inTex.sample(s, uv + float2(-2.0 * texel.x, 0.0)).rgb;
        float3 q21 = inTex.sample(s, uv + float2( 2.0 * texel.x, 0.0)).rgb;
        float3 q10 = inTex.sample(s, uv + float2(0.0, -2.0 * texel.y)).rgb;
        float3 q12 = inTex.sample(s, uv + float2(0.0,  2.0 * texel.y)).rgb;

        float skin = skin_likelihood(p11);

        // 1. FidelityFX CAS (fine detail: eyelashes, hair, fabric)
        float3 mnRGB = min(min(min(p01, p11), min(p21, p10)), p12);
        mnRGB += min(mnRGB, min(min(p00, p20), min(p02, p22)));
        float3 mxRGB = max(max(max(p01, p11), max(p21, p10)), p12);
        mxRGB += max(mxRGB, max(max(p00, p20), max(p02, p22)));

        float3 rcpMRGB = 1.0 / max(mxRGB, float3(0.0001));
        float3 ampRGB = sqrt(clamp(min(mnRGB, 2.0 - mxRGB) * rcpMRGB, 0.0, 1.0));
        float peak = -1.0 / mix(7.2, 3.6, clamp(0.68 * strength * (1.0 - 0.45 * skin), 0.1, 1.0));
        float3 wRGB = ampRGB * peak;
        float3 casRGB = clamp(((p10 + p01 + p21 + p12) * wRGB + p11) / (1.0 + 4.0 * wRGB), 0.0, 1.0);

        // 2. Portrait-safe 2-texel local clarity (boosts texture depth without harsh skin pores)
        float3 blur2 = (q10 + q12 + q01 + q21) * 0.25;
        float3 localDetail = p11 - blur2;
        float clarityAmt = mix(0.42, 0.14, skin) * strength;
        float3 detailed = clamp(casRGB + localDetail * clarityAmt, 0.0, 1.0);

        // 3. Filmic tone & shadow lift + perceptual contrast S-curve
        float lumaVal = luma(detailed);
        float shadowLift = smoothstep(0.32, 0.02, lumaVal) * 0.045 * strength;
        float3 lifted = clamp(detailed + float3(shadowLift), 0.0, 1.0);
        float3 sCurve = lifted * lifted * (3.0 - 2.0 * lifted);
        float3 toned = mix(lifted, sCurve, clamp(0.22 * strength, 0.0, 0.36));

        // 4. Smart vibrance (lifts muted colors while protecting skin tones)
        float maxC = max(toned.r, max(toned.g, toned.b));
        float minC = min(toned.r, min(toned.g, toned.b));
        float sat = maxC - minC;
        float vibAmt = (1.0 - smoothstep(0.25, 0.72, sat)) * (1.0 - 0.65 * skin) * 0.20 * strength;
        float lToned = luma(toned);
        float3 finalRGB = clamp(mix(float3(lToned), toned, 1.0 + vibAmt), 0.0, 1.0);

        outTex.write(float4(finalRGB, 1.0), gid);
    }
    """

    private init() {
        cache.countLimit = 28
        guard let dev = MTLCreateSystemDefaultDevice(),
              let q = dev.makeCommandQueue() else {
            NSLog("[MetalImageEnhancer] Metal device or queue unavailable")
            self.device = nil
            self.queue = nil
            self.animePipeline = nil
            self.photoPipeline = nil
            return
        }
        do {
            let lib = try dev.makeLibrary(source: Self.shaderSource, options: nil)
            self.device = dev
            self.queue = q
            self.animePipeline = lib.makeFunction(name: "enhance_anime").flatMap { try? dev.makeComputePipelineState(function: $0) }
            self.photoPipeline = lib.makeFunction(name: "enhance_photo").flatMap { try? dev.makeComputePipelineState(function: $0) }
        } catch {
            NSLog("[MetalImageEnhancer] Shader compilation failed: %@", String(describing: error))
            self.device = nil
            self.queue = nil
            self.animePipeline = nil
            self.photoPipeline = nil
        }
    }

    func cachedResult(cacheKey: String, mode: ReaderEnhanceMode, strength: ReaderEnhanceStrength, autoCrop: Bool) -> UIImage? {
        if mode == .off && !autoCrop { return nil }
        let fullKey = "\(mode.rawValue)|\(strength.rawValue)|\(autoCrop ? 1 : 0)|\(cacheKey)" as NSString
        return cache.object(forKey: fullKey)
    }

    func process(
        image: UIImage,
        cacheKey: String,
        mode: ReaderEnhanceMode,
        strength: ReaderEnhanceStrength = .standard,
        autoCrop: Bool
    ) -> UIImage {
        if mode == .off && !autoCrop { return image }
        let fullKey = "\(mode.rawValue)|\(strength.rawValue)|\(autoCrop ? 1 : 0)|\(cacheKey)" as NSString
        if let hit = cache.object(forKey: fullKey) { return hit }

        var working = image
        if autoCrop, let cropped = detectAndCrop(working) {
            working = cropped
        }
        if mode != .off, let enhanced = runMetal(working, mode: mode, strength: strength) {
            working = enhanced
        }
        cache.setObject(working, forKey: fullKey)
        return working
    }

    private func detectAndCrop(_ image: UIImage) -> UIImage? {
        guard let cg = image.cgImage else { return nil }
        let natW = cg.width
        let natH = cg.height
        guard natW >= 80, natH >= 80 else { return nil }

        let scale = min(1.0, 256.0 / Double(max(natW, natH)))
        let sw = max(16, Int(Double(natW) * scale))
        let sh = max(16, Int(Double(natH) * scale))
        var pixels = [UInt8](repeating: 0, count: sw * sh * 4)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        guard let ctx = CGContext(
            data: &pixels,
            width: sw,
            height: sh,
            bitsPerComponent: 8,
            bytesPerRow: sw * 4,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: sw, height: sh))

        func lum(_ idx: Int) -> Double {
            0.299 * Double(pixels[idx]) + 0.587 * Double(pixels[idx + 1]) + 0.114 * Double(pixels[idx + 2])
        }
        func chroma(_ idx: Int) -> Int {
            let r = Int(pixels[idx]), g = Int(pixels[idx + 1]), b = Int(pixels[idx + 2])
            return max(r, max(g, b)) - min(r, min(g, b))
        }

        let corners = [0, (sw - 1) * 4, (sh - 1) * sw * 4, ((sh - 1) * sw + (sw - 1)) * 4]
        let avgCorner = corners.reduce(0.0) { $0 + lum($1) } / 4.0
        let darkMode = avgCorner < 22
        let lightMode = avgCorner > 236
        if !darkMode && !lightMode { return nil }

        func isBorder(_ idx: Int) -> Bool {
            let l = lum(idx)
            let ch = chroma(idx)
            return darkMode ? (l < 18 && ch < 16) : (l > 240 && ch < 18)
        }

        let maxX = Int(Double(sw) * 0.18)
        let maxY = Int(Double(sh) * 0.18)

        func rowIsBorder(_ y: Int) -> Bool {
            var cnt = 0
            for x in 0..<sw where isBorder((y * sw + x) * 4) { cnt += 1 }
            return Double(cnt) / Double(sw) > 0.96
        }
        func colIsBorder(_ x: Int, _ y0: Int, _ y1: Int) -> Bool {
            var cnt = 0
            let h = max(1, y1 - y0)
            for y in y0..<y1 where isBorder((y * sw + x) * 4) { cnt += 1 }
            return Double(cnt) / Double(h) > 0.96
        }

        var top = 0; while top < maxY && rowIsBorder(top) { top += 1 }
        var bottom = sh - 1; while bottom > sh - 1 - maxY && rowIsBorder(bottom) { bottom -= 1 }
        var left = 0; while left < maxX && colIsBorder(left, top, bottom + 1) { left += 1 }
        var right = sw - 1; while right > sw - 1 - maxX && colIsBorder(right, top, bottom + 1) { right -= 1 }

        if left < 2 && right > sw - 3 && top < 2 && bottom > sh - 3 { return nil }

        let rx = Int(Double(left) / Double(sw) * Double(natW))
        let ry = Int(Double(top) / Double(sh) * Double(natH))
        let rw = max(64, Int(Double(right - left + 1) / Double(sw) * Double(natW)))
        let rh = max(64, Int(Double(bottom - top + 1) / Double(sh) * Double(natH)))
        let rect = CGRect(x: rx, y: ry, width: min(natW - rx, rw), height: min(natH - ry, rh))
        guard let outCG = cg.cropping(to: rect) else { return nil }
        return UIImage(cgImage: outCG, scale: image.scale, orientation: image.imageOrientation)
    }

    private func runMetal(_ image: UIImage, mode: ReaderEnhanceMode, strength: ReaderEnhanceStrength) -> UIImage? {
        guard let dev = device,
              let queue = queue,
              let pipeline = (mode == .anime ? animePipeline : photoPipeline),
              let cg = image.cgImage else { return nil }

        let srcW = cg.width
        let srcH = cg.height
        let maxDim = mode == .anime ? 2800 : 3072
        let scale: Double = (mode == .anime && max(srcW, srcH) < 2200)
            ? min(1.8, Double(maxDim) / Double(max(srcW, srcH)))
            : min(1.0, Double(maxDim) / Double(max(srcW, srcH)))
        let outW = max(1, Int(Double(srcW) * scale))
        let outH = max(1, Int(Double(srcH) * scale))

        var inBytes = [UInt8](repeating: 0, count: srcW * srcH * 4)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        guard let inCtx = CGContext(
            data: &inBytes,
            width: srcW,
            height: srcH,
            bitsPerComponent: 8,
            bytesPerRow: srcW * 4,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        inCtx.draw(cg, in: CGRect(x: 0, y: 0, width: srcW, height: srcH))

        let inDesc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba8Unorm, width: srcW, height: srcH, mipmapped: false)
        inDesc.usage = [.shaderRead]
        guard let inTex = dev.makeTexture(descriptor: inDesc) else { return nil }
        inTex.replace(region: MTLRegionMake2D(0, 0, srcW, srcH), mipmapLevel: 0, withBytes: inBytes, bytesPerRow: srcW * 4)

        let outDesc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba8Unorm, width: outW, height: outH, mipmapped: false)
        outDesc.usage = [.shaderWrite, .shaderRead]
        guard let outTex = dev.makeTexture(descriptor: outDesc),
              let cmdBuf = queue.makeCommandBuffer(),
              let enc = cmdBuf.makeComputeCommandEncoder() else { return nil }

        var strengthVal = strength.factor
        enc.setComputePipelineState(pipeline)
        enc.setTexture(inTex, index: 0)
        enc.setTexture(outTex, index: 1)
        enc.setBytes(&strengthVal, length: MemoryLayout<Float>.size, index: 0)
        let tgW = pipeline.threadExecutionWidth
        let tgH = max(1, pipeline.maxTotalThreadsPerThreadgroup / tgW)
        let threadsPerGroup = MTLSize(width: tgW, height: tgH, depth: 1)
        let threadsPerGrid = MTLSize(width: outW, height: outH, depth: 1)
        enc.dispatchThreads(threadsPerGrid, threadsPerThreadgroup: threadsPerGroup)
        enc.endEncoding()
        cmdBuf.commit()
        cmdBuf.waitUntilCompleted()

        var outBytes = [UInt8](repeating: 0, count: outW * outH * 4)
        outTex.getBytes(&outBytes, bytesPerRow: outW * 4, from: MTLRegionMake2D(0, 0, outW, outH), mipmapLevel: 0)
        guard let outCtx = CGContext(
            data: &outBytes,
            width: outW,
            height: outH,
            bitsPerComponent: 8,
            bytesPerRow: outW * 4,
            space: colorSpace,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ), let outCG = outCtx.makeImage() else { return nil }
        return UIImage(cgImage: outCG)
    }
}

// MARK: - Standalone Zero-Chrome Minimalist Comic & Gallery Reader

struct ComicReaderChapterItem: Identifiable, Hashable {
    let id: String
    let title: String
    let active: Bool
}

struct ComicGalleryReaderView: View {
    let kind: ReaderKind
    let title: String
    let sources: [ReaderPageSource]
    var initialPage: Int = 0
    var chapters: [ComicReaderChapterItem] = []
    var onSelectChapter: ((String) -> Void)? = nil
    var onDismiss: (() -> Void)? = nil

    init(
        kind: ReaderKind,
        title: String,
        sources: [ReaderPageSource],
        initialPage: Int = 0,
        chapters: [ComicReaderChapterItem] = [],
        onSelectChapter: ((String) -> Void)? = nil,
        onDismiss: (() -> Void)? = nil
    ) {
        self.kind = kind
        self.title = title
        self.sources = sources
        self.initialPage = initialPage
        self.chapters = chapters
        self.onSelectChapter = onSelectChapter
        self.onDismiss = onDismiss
        let clamped = sources.isEmpty ? 0 : min(max(0, initialPage), sources.count - 1)
        _currentIndex = State(initialValue: clamped)
        _sliderValue = State(initialValue: Double(clamped + 1))
    }

    init(
        kind: ReaderKind,
        title: String,
        pageURLs: [URL],
        initialPage: Int = 0,
        chapters: [ComicReaderChapterItem] = [],
        onSelectChapter: ((String) -> Void)? = nil,
        onDismiss: (() -> Void)? = nil
    ) {
        self.init(
            kind: kind,
            title: title,
            sources: pageURLs.map { .url($0) },
            initialPage: initialPage,
            chapters: chapters,
            onSelectChapter: onSelectChapter,
            onDismiss: onDismiss
        )
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var readMode: ReaderLayoutMode = .strip
    @State private var enhanceMode: ReaderEnhanceMode = .off
    @State private var enhanceStrength: ReaderEnhanceStrength = .standard
    @State private var autoCrop: Bool = false
    @State private var warmBg: Bool = false

    @State private var currentIndex: Int = 0
    @State private var sliderValue: Double = 1
    @State private var chromeVisible: Bool = false
    @State private var activePopover: PopoverKind? = nil
    @State private var peekingOriginal: Bool = false
    @State private var currentZoomScale: CGFloat = 1.0

    // Strip mode pinch-to-zoom & pan state
    @State private var stripZoom: CGFloat = 1.0
    @State private var stripBaseZoom: CGFloat = 1.0
    @State private var stripPan: CGSize = .zero
    @State private var stripBasePan: CGSize = .zero

    private enum PopoverKind { case mode, enhance, display, chapters }

    private var prefsPrefix: String { "pml.ios.reader.\(kind.rawValue)." }

    /// 仅在打开「画质增强」设置面板时才允许长按对比原图，平时阅读不误触
    private var canPeekOriginal: Bool {
        chromeVisible && activePopover == .enhance && (enhanceMode != .off || autoCrop)
    }

    var body: some View {
        ZStack {
            (warmBg ? Color(red: 0.96, green: 0.95, blue: 0.92) : Color.black)
                .ignoresSafeArea()

            if sources.isEmpty {
                ProgressView("加载页面中…")
                    .tint(.white)
                    .foregroundStyle(.secondary)
            } else if readMode == .strip {
                stripCanvas
            } else {
                pagedCanvas
            }

            // 极淡角落页码（沉浸态）
            if !chromeVisible && !sources.isEmpty {
                VStack {
                    Spacer()
                    HStack {
                        Spacer()
                        Text("\(currentIndex + 1) / \(sources.count)")
                            .font(.system(size: 11, weight: .semibold, design: .rounded))
                            .foregroundStyle(warmBg ? Color.black.opacity(0.42) : Color.white.opacity(0.42))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 3)
                            .background((warmBg ? Color.white : Color.black).opacity(0.32), in: Capsule())
                            .padding(.trailing, 12)
                            .padding(.bottom, 8)
                    }
                }
                .allowsHitTesting(false)
            }

            // 长按原图对比胶囊
            if peekingOriginal {
                VStack {
                    Text("原图对比中")
                        .font(.caption.bold())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 6)
                        .background(Color.black.opacity(0.78), in: Capsule())
                        .overlay(Capsule().stroke(Color.cyan.opacity(0.65), lineWidth: 1))
                        .padding(.top, 56)
                    Spacer()
                }
                .allowsHitTesting(false)
                .transition(.opacity)
            }

            // 唤出态：毛玻璃顶栏 + 轻量设置弹层 + 毛玻璃底栏
            if chromeVisible {
                VStack(spacing: 0) {
                    topBar
                    Spacer()
                    if let pop = activePopover {
                        popoverCard(pop)
                            .padding(.horizontal, 16)
                            .padding(.bottom, 10)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                    bottomBar
                }
                .transition(.opacity)
            }
        }
        .statusBarHidden(!chromeVisible)
        .onAppear { loadPrefs() }
        .task(id: currentIndex) {
            await prefetchNeighbors(around: currentIndex)
        }
        .onChange(of: activePopover) { _, newVal in
            if newVal != .enhance { peekingOriginal = false }
        }
        .onChange(of: chromeVisible) { _, vis in
            if !vis { peekingOriginal = false }
        }
    }

    private var stripCanvas: some View {
        GeometryReader { geo in
            ScrollViewReader { proxy in
                ScrollView(stripZoom > 1.04 ? [] : .vertical) {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(sources.enumerated()), id: \.element.id) { idx, src in
                            EnhancedReaderPageView(
                                source: src,
                                enhanceMode: peekingOriginal ? .off : enhanceMode,
                                enhanceStrength: enhanceStrength,
                                autoCrop: peekingOriginal ? false : autoCrop
                            )
                            .id(idx)
                            .onAppear {
                                currentIndex = idx
                                sliderValue = Double(idx + 1)
                            }
                        }
                    }
                    .scaleEffect(stripZoom, anchor: .center)
                    .offset(stripPan)
                }
                .contentShape(Rectangle())
                .simultaneousGesture(
                    MagnificationGesture()
                        .onChanged { value in
                            let next = min(4.0, max(1.0, stripBaseZoom * value))
                            stripZoom = next
                            if next <= 1.02 {
                                stripPan = .zero
                            }
                        }
                        .onEnded { _ in
                            if stripZoom <= 1.05 {
                                withAnimation(.easeOut(duration: 0.18)) {
                                    stripZoom = 1.0
                                    stripBaseZoom = 1.0
                                    stripPan = .zero
                                    stripBasePan = .zero
                                }
                            } else {
                                stripBaseZoom = stripZoom
                            }
                        }
                )
                .simultaneousGesture(
                    DragGesture(minimumDistance: 6)
                        .onChanged { value in
                            guard stripZoom > 1.05 else { return }
                            let maxX = geo.size.width * (stripZoom - 1.0) * 0.55
                            let maxY = geo.size.height * (stripZoom - 1.0) * 0.65
                            stripPan = CGSize(
                                width: min(maxX, max(-maxX, stripBasePan.width + value.translation.width)),
                                height: min(maxY, max(-maxY, stripBasePan.height + value.translation.height))
                            )
                        }
                        .onEnded { _ in
                            stripBasePan = stripPan
                        },
                    including: stripZoom > 1.05 ? .all : .subviews
                )
                .onTapGesture(count: 2) {
                    withAnimation(.easeInOut(duration: 0.22)) {
                        if stripZoom > 1.1 {
                            stripZoom = 1.0
                            stripBaseZoom = 1.0
                            stripPan = .zero
                            stripBasePan = .zero
                        } else {
                            stripZoom = 2.2
                            stripBaseZoom = 2.2
                        }
                    }
                }
                .onTapGesture {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        if activePopover != nil {
                            activePopover = nil
                        } else {
                            chromeVisible.toggle()
                        }
                    }
                }
                .simultaneousGesture(
                    LongPressGesture(minimumDuration: 0.25)
                        .sequenced(before: DragGesture(minimumDistance: 0))
                        .onChanged { phase in
                            guard canPeekOriginal else { return }
                            if case .second(true, _) = phase {
                                peekingOriginal = true
                            }
                        }
                        .onEnded { _ in
                            peekingOriginal = false
                        },
                    including: canPeekOriginal ? .all : .subviews
                )
                .onChange(of: sliderValue) { _, newVal in
                    let target = max(0, min(sources.count - 1, Int(newVal) - 1))
                    if abs(target - currentIndex) > 1 {
                        proxy.scrollTo(target, anchor: .top)
                    }
                }
                .onAppear {
                    if initialPage > 0, initialPage < sources.count {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                            proxy.scrollTo(initialPage, anchor: .top)
                        }
                    }
                }
            }
        }
    }

    private var pagedCanvas: some View {
        TabView(selection: $currentIndex) {
            if readMode == .double {
                ForEach(doublePageStarts, id: \.self) { startIdx in
                    HStack(spacing: 2) {
                        EnhancedReaderPageView(
                            source: sources[startIdx],
                            enhanceMode: peekingOriginal ? .off : enhanceMode,
                            enhanceStrength: enhanceStrength,
                            autoCrop: peekingOriginal ? false : autoCrop
                        )
                        if startIdx + 1 < sources.count {
                            EnhancedReaderPageView(
                                source: sources[startIdx + 1],
                                enhanceMode: peekingOriginal ? .off : enhanceMode,
                                enhanceStrength: enhanceStrength,
                                autoCrop: peekingOriginal ? false : autoCrop
                            )
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .onTapGesture { toggleChromeOrClosePopover() }
                    .tag(startIdx)
                }
            } else {
                ForEach(Array(sources.enumerated()), id: \.element.id) { idx, src in
                    ZoomableReaderPageView(
                        source: src,
                        enhanceMode: peekingOriginal ? .off : enhanceMode,
                        enhanceStrength: enhanceStrength,
                        autoCrop: peekingOriginal ? false : autoCrop,
                        warmBg: warmBg,
                        canLongPress: canPeekOriginal,
                        onSingleTap: { toggleChromeOrClosePopover() },
                        onLongPressChanged: { pressing in
                            if canPeekOriginal {
                                peekingOriginal = pressing
                            } else if !pressing {
                                peekingOriginal = false
                            }
                        },
                        onZoomScaleChanged: { scale in
                            currentZoomScale = scale
                        }
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .tag(idx)
                }
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        .environment(\.layoutDirection, readMode == .manga ? .rightToLeft : .leftToRight)
        .onChange(of: currentIndex) { _, newVal in
            sliderValue = Double(newVal + 1)
            currentZoomScale = 1.0
        }
    }

    private var doublePageStarts: [Int] {
        stride(from: 0, to: sources.count, by: 2).map { $0 }
    }

    private func toggleChromeOrClosePopover() {
        withAnimation(.easeInOut(duration: 0.2)) {
            if activePopover != nil {
                activePopover = nil
            } else {
                chromeVisible.toggle()
            }
        }
    }

    private var topBar: some View {
        HStack(spacing: 12) {
            Button {
                if let onDismiss { onDismiss() } else { dismiss() }
            } label: {
                Label("返回", systemImage: "chevron.left")
                    .font(.subheadline.weight(.semibold))
                    .padding(.horizontal, 11)
                    .padding(.vertical, 6)
                    .background(.white.opacity(0.12), in: Capsule())
            }
            Text(title)
                .font(.subheadline.weight(.bold))
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .center)
            if chapters.count > 1 {
                Button {
                    withAnimation(.easeInOut(duration: 0.18)) {
                        activePopover = activePopover == .chapters ? nil : .chapters
                    }
                } label: {
                    Label("目录", systemImage: "list.bullet")
                        .font(.subheadline.weight(.semibold))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(.white.opacity(0.12), in: Capsule())
                }
            } else {
                Color.clear.frame(width: 58, height: 1)
            }
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    private var bottomBar: some View {
        VStack(spacing: 10) {
            if sources.count > 1 {
                HStack(spacing: 10) {
                    Text("\(Int(sliderValue))")
                        .font(.caption.monospacedDigit())
                        .frame(minWidth: 30)
                    Slider(
                        value: $sliderValue,
                        in: 1...Double(max(1, sources.count)),
                        step: 1
                    ) { editing in
                        if !editing {
                            currentIndex = max(0, min(sources.count - 1, Int(sliderValue) - 1))
                        }
                    }
                    .tint(.cyan)
                    Text("\(sources.count)")
                        .font(.caption.monospacedDigit())
                        .frame(minWidth: 30)
                }
                .foregroundStyle(.white.opacity(0.85))
            }

            HStack(spacing: 10) {
                pillButton(title: readMode.shortLabel, icon: "book", active: activePopover == .mode) {
                    withAnimation(.easeInOut(duration: 0.18)) {
                        activePopover = activePopover == .mode ? nil : .mode
                    }
                }
                pillButton(title: enhanceMode.shortLabel, icon: "sparkles", active: activePopover == .enhance || enhanceMode != .off) {
                    withAnimation(.easeInOut(duration: 0.18)) {
                        activePopover = activePopover == .enhance ? nil : .enhance
                    }
                }
                pillButton(title: "显示", icon: "slider.horizontal.3", active: activePopover == .display || autoCrop) {
                    withAnimation(.easeInOut(duration: 0.18)) {
                        activePopover = activePopover == .display ? nil : .display
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .padding(.bottom, 16)
        .background(.ultraThinMaterial)
    }

    private func pillButton(title: String, icon: String, active: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: icon)
                .font(.caption.weight(.semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 9)
                .background(active ? Color.cyan.opacity(0.28) : Color.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
                .foregroundStyle(active ? .cyan : .white)
        }
    }

    @ViewBuilder
    private func popoverCard(_ pop: PopoverKind) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            switch pop {
            case .mode:
                Text("阅读模式").font(.caption.bold()).foregroundStyle(.white.opacity(0.7))
                HStack(spacing: 6) {
                    ForEach(ReaderLayoutMode.allCases) { m in
                        Button(m.shortLabel) {
                            readMode = m
                            stripZoom = 1.0
                            stripBaseZoom = 1.0
                            stripPan = .zero
                            stripBasePan = .zero
                            savePrefs()
                            activePopover = nil
                        }
                        .font(.caption.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .background(readMode == m ? Color.cyan.opacity(0.3) : Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 9))
                        .foregroundStyle(.white)
                    }
                }
            case .enhance:
                HStack {
                    Text("Metal GPU 画质增强").font(.caption.bold()).foregroundStyle(.white.opacity(0.7))
                    Spacer()
                    if enhanceMode != .off || autoCrop {
                        Text("调整时可长按画面对比原图")
                            .font(.caption2)
                            .foregroundStyle(.cyan)
                    }
                }
                HStack(spacing: 6) {
                    ForEach(ReaderEnhanceMode.allCases) { m in
                        Button(m.label) {
                            enhanceMode = m
                            savePrefs()
                        }
                        .font(.caption.weight(.semibold))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .background(enhanceMode == m ? Color.cyan.opacity(0.3) : Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 9))
                        .foregroundStyle(.white)
                    }
                }
                if enhanceMode != .off {
                    HStack(spacing: 6) {
                        Text("增强强度")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.white.opacity(0.7))
                        ForEach(ReaderEnhanceStrength.allCases) { st in
                            Button(st.label) {
                                enhanceStrength = st
                                savePrefs()
                            }
                            .font(.caption2.weight(.semibold))
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 6)
                            .background(enhanceStrength == st ? Color.cyan.opacity(0.3) : Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
                            .foregroundStyle(.white)
                        }
                    }
                }
                if enhanceMode != .off || autoCrop {
                    HStack {
                        Label(peekingOriginal ? "正在显示原图…" : "按住此处或长按画面对比原图", systemImage: "eye")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(peekingOriginal ? .black : .cyan)
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 8)
                            .background(peekingOriginal ? Color.cyan : Color.cyan.opacity(0.16), in: RoundedRectangle(cornerRadius: 9))
                            .gesture(
                                DragGesture(minimumDistance: 0)
                                    .onChanged { _ in peekingOriginal = true }
                                    .onEnded { _ in peekingOriginal = false }
                            )
                    }
                }
            case .display:
                Toggle(isOn: $autoCrop) {
                    Text("智能切白边").font(.subheadline.weight(.semibold)).foregroundStyle(.white)
                }
                .tint(.cyan)
                .onChange(of: autoCrop) { _, _ in savePrefs() }

                HStack {
                    Text("画布背景").font(.subheadline.weight(.semibold)).foregroundStyle(.white)
                    Spacer()
                    HStack(spacing: 6) {
                        Button("纯黑") { warmBg = false; savePrefs() }
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(!warmBg ? Color.cyan.opacity(0.3) : Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
                            .foregroundStyle(.white)
                        Button("暖白") { warmBg = true; savePrefs() }
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(warmBg ? Color.cyan.opacity(0.3) : Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
                            .foregroundStyle(.white)
                    }
                }
            case .chapters:
                Text("章节目录 (\(chapters.count))").font(.caption.bold()).foregroundStyle(.white.opacity(0.7))
                ScrollView {
                    VStack(spacing: 6) {
                        ForEach(chapters) { ch in
                            Button {
                                activePopover = nil
                                onSelectChapter?(ch.id)
                            } label: {
                                HStack {
                                    Text(ch.title)
                                        .font(.caption.weight(.semibold))
                                        .lineLimit(1)
                                    Spacer()
                                    if ch.active {
                                        Image(systemName: "checkmark").font(.caption2)
                                    }
                                }
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
                                .background(ch.active ? Color.cyan.opacity(0.28) : Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 9))
                                .foregroundStyle(.white)
                            }
                        }
                    }
                }
                .frame(maxHeight: 210)
            }
        }
        .padding(14)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 18))
        .overlay(RoundedRectangle(cornerRadius: 18).stroke(Color.white.opacity(0.14), lineWidth: 1))
    }

    private func prefetchNeighbors(around center: Int) async {
        for offset in [1, 2, 3, -1] {
            let idx = center + offset
            guard sources.indices.contains(idx) else { continue }
            switch sources[idx] {
            case .url(let u):
                Task { _ = await ImageCache.shared.cachedOrLoad(u, maxPixel: 2048) }
            case .comicArchivePage(let aid, let pIdx, let fallbackURL):
                if let direct = model.apiClient != nil ? ComicDirectStore.shared.cachedImage(assetId: aid, index: pIdx) : nil {
                    _ = direct
                } else {
                    ComicDirectStore.shared.prefetch(assetId: aid, index: pIdx, model: model)
                    Task { _ = await ImageCache.shared.cachedOrLoad(fallbackURL, maxPixel: 2048) }
                }
            case .remoteAsset(let aid, let rp, let fallbackURL):
                Task {
                    if let resolved = await model.imageSource(assetId: aid, remotePath: rp)?.url {
                        _ = await ImageCache.shared.cachedOrLoad(resolved, maxPixel: 2048)
                    } else {
                        _ = await ImageCache.shared.cachedOrLoad(fallbackURL, maxPixel: 2048)
                    }
                }
            }
        }
    }

    private func loadPrefs() {
        let defaults = UserDefaults.standard
        if let rawMode = defaults.string(forKey: prefsPrefix + "mode"),
           let m = ReaderLayoutMode(rawValue: rawMode) {
            readMode = m
        } else {
            readMode = kind == .gallery ? .single : .strip
        }
        if let rawEnh = defaults.string(forKey: prefsPrefix + "enhance"),
           let e = ReaderEnhanceMode(rawValue: rawEnh) {
            enhanceMode = e
        } else {
            enhanceMode = kind == .gallery ? .photo : .anime
        }
        if let rawStr = defaults.string(forKey: prefsPrefix + "strength"),
           let s = ReaderEnhanceStrength(rawValue: rawStr) {
            enhanceStrength = s
        } else {
            enhanceStrength = .standard
        }
        autoCrop = defaults.bool(forKey: prefsPrefix + "crop")
        warmBg = defaults.bool(forKey: prefsPrefix + "warm")
    }

    private func savePrefs() {
        let defaults = UserDefaults.standard
        defaults.set(readMode.rawValue, forKey: prefsPrefix + "mode")
        defaults.set(enhanceMode.rawValue, forKey: prefsPrefix + "enhance")
        defaults.set(enhanceStrength.rawValue, forKey: prefsPrefix + "strength")
        defaults.set(autoCrop, forKey: prefsPrefix + "crop")
        defaults.set(warmBg, forKey: prefsPrefix + "warm")
    }
}

// MARK: - Progressive Raw-First + Animated Enhancement Reveal

private enum ReaderSourceLoader {
    @MainActor
    static func loadRaw(source: ReaderPageSource, model: AppModel) async -> UIImage? {
        switch source {
        case .url(let u):
            return await ImageCache.shared.cachedOrLoad(u, maxPixel: 2048)
        case .comicArchivePage(let aid, let pIdx, let fallbackURL):
            if let direct = await ComicDirectStore.shared.image(assetId: aid, index: pIdx, model: model) {
                return direct
            }
            return await ImageCache.shared.cachedOrLoad(fallbackURL, maxPixel: 2048)
        case .remoteAsset(let aid, let rp, let fallbackURL):
            if !aid.isEmpty, let resolved = await model.imageSource(assetId: aid, remotePath: rp)?.url {
                if let img = await ImageCache.shared.cachedOrLoad(resolved, maxPixel: 2048) {
                    return img
                }
            }
            if let direct = model.alistDirectURL(remotePath: rp),
               let img = await ImageCache.shared.cachedOrLoad(direct, maxPixel: 2048) {
                return img
            }
            return await ImageCache.shared.cachedOrLoad(fallbackURL, maxPixel: 2048)
        }
    }
}

private struct EnhanceRevealSweepOverlay: View {
    @State private var sweepProgress: CGFloat = -0.35

    var body: some View {
        GeometryReader { geo in
            LinearGradient(
                colors: [.clear, Color.cyan.opacity(0.28), Color.white.opacity(0.32), .clear],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .frame(width: geo.size.width * 0.48, height: geo.size.height * 1.4)
            .rotationEffect(.degrees(14))
            .offset(x: sweepProgress * geo.size.width * 1.4)
            .onAppear {
                withAnimation(.easeInOut(duration: 0.65)) {
                    sweepProgress = 1.15
                }
            }
        }
        .clipped()
        .allowsHitTesting(false)
    }
}

private struct EnhancedReaderPageView: View {
    let source: ReaderPageSource
    let enhanceMode: ReaderEnhanceMode
    let enhanceStrength: ReaderEnhanceStrength
    let autoCrop: Bool

    @EnvironmentObject private var model: AppModel
    @State private var rawImage: UIImage?
    @State private var displayImage: UIImage?
    @State private var showRevealAnimation = false

    private var enhanceKey: String {
        "\(source.cacheKey)|\(enhanceMode.rawValue)|\(enhanceStrength.rawValue)|\(autoCrop ? 1 : 0)"
    }

    var body: some View {
        ZStack {
            if let img = displayImage ?? rawImage {
                Image(uiImage: img)
                    .resizable()
                    .scaledToFit()
                    .transition(.opacity)
            } else {
                ProgressView()
                    .tint(.white.opacity(0.6))
                    .frame(maxWidth: .infinity, minHeight: 320)
            }

            if showRevealAnimation {
                EnhanceRevealSweepOverlay()
                VStack {
                    HStack {
                        Spacer()
                        Label("已增强", systemImage: "sparkles")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(Color.black.opacity(0.68), in: Capsule())
                            .overlay(Capsule().stroke(Color.cyan.opacity(0.55), lineWidth: 1))
                            .padding(10)
                    }
                    Spacer()
                }
                .transition(.opacity)
                .allowsHitTesting(false)
            }
        }
        .task(id: source.id) {
            if let ui = await ReaderSourceLoader.loadRaw(source: source, model: model) {
                rawImage = ui
                await applyEnhancement(ui)
            }
        }
        .task(id: enhanceKey) {
            if let ui = rawImage {
                await applyEnhancement(ui)
            }
        }
    }

    private func applyEnhancement(_ base: UIImage) async {
        if enhanceMode == .off && !autoCrop {
            displayImage = base
            showRevealAnimation = false
            return
        }
        if let hit = MetalImageEnhancer.shared.cachedResult(
            cacheKey: source.cacheKey,
            mode: enhanceMode,
            strength: enhanceStrength,
            autoCrop: autoCrop
        ) {
            displayImage = hit
            return
        }
        try? await Task.sleep(for: .milliseconds(70))
        guard !Task.isCancelled else { return }
        let key = source.cacheKey
        let m = enhanceMode
        let st = enhanceStrength
        let c = autoCrop
        let processed = await Task.detached(priority: .utility) {
            MetalImageEnhancer.shared.process(image: base, cacheKey: key, mode: m, strength: st, autoCrop: c)
        }.value
        guard !Task.isCancelled else { return }
        withAnimation(.easeInOut(duration: 0.30)) {
            displayImage = processed
            showRevealAnimation = true
        }
        try? await Task.sleep(for: .milliseconds(1100))
        if !Task.isCancelled {
            withAnimation(.easeOut(duration: 0.3)) {
                showRevealAnimation = false
            }
        }
    }
}

private struct ZoomableReaderPageRepresentable: UIViewRepresentable {
    let image: UIImage
    let warmBg: Bool
    let canLongPress: Bool
    let onSingleTap: () -> Void
    let onLongPressChanged: (Bool) -> Void
    let onZoomScaleChanged: (CGFloat) -> Void

    func makeUIView(context: Context) -> ZoomScrollView {
        let view = ZoomScrollView()
        view.backgroundColor = warmBg ? UIColor(red: 0.96, green: 0.95, blue: 0.92, alpha: 1.0) : .black
        view.canLongPress = canLongPress
        view.onSingleTap = onSingleTap
        view.onLongPressChanged = onLongPressChanged
        view.onZoomScaleChanged = onZoomScaleChanged
        view.updateImage(image, preserveZoom: false, animated: false)
        return view
    }

    func updateUIView(_ view: ZoomScrollView, context: Context) {
        view.backgroundColor = warmBg ? UIColor(red: 0.96, green: 0.95, blue: 0.92, alpha: 1.0) : .black
        view.canLongPress = canLongPress
        view.onSingleTap = onSingleTap
        view.onLongPressChanged = onLongPressChanged
        view.onZoomScaleChanged = onZoomScaleChanged
        view.updateImage(image, preserveZoom: true, animated: true)
    }
}

private struct ZoomableReaderPageView: View {
    let source: ReaderPageSource
    let enhanceMode: ReaderEnhanceMode
    let enhanceStrength: ReaderEnhanceStrength
    let autoCrop: Bool
    let warmBg: Bool
    let canLongPress: Bool
    let onSingleTap: () -> Void
    let onLongPressChanged: (Bool) -> Void
    let onZoomScaleChanged: (CGFloat) -> Void

    @EnvironmentObject private var model: AppModel
    @State private var rawImage: UIImage?
    @State private var displayImage: UIImage?
    @State private var showRevealAnimation = false

    private var enhanceKey: String {
        "\(source.cacheKey)|\(enhanceMode.rawValue)|\(enhanceStrength.rawValue)|\(autoCrop ? 1 : 0)"
    }

    var body: some View {
        ZStack {
            if let img = displayImage ?? rawImage {
                ZoomableReaderPageRepresentable(
                    image: img,
                    warmBg: warmBg,
                    canLongPress: canLongPress,
                    onSingleTap: onSingleTap,
                    onLongPressChanged: onLongPressChanged,
                    onZoomScaleChanged: onZoomScaleChanged
                )
            } else {
                ProgressView()
                    .tint(.white.opacity(0.6))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .onTapGesture { onSingleTap() }
            }

            if showRevealAnimation {
                EnhanceRevealSweepOverlay()
                VStack {
                    HStack {
                        Spacer()
                        Label("已增强", systemImage: "sparkles")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 4)
                            .background(Color.black.opacity(0.68), in: Capsule())
                            .overlay(Capsule().stroke(Color.cyan.opacity(0.55), lineWidth: 1))
                            .padding(.top, 56)
                            .padding(.trailing, 14)
                    }
                    Spacer()
                }
                .transition(.opacity)
                .allowsHitTesting(false)
            }
        }
        .task(id: source.id) {
            if let ui = await ReaderSourceLoader.loadRaw(source: source, model: model) {
                rawImage = ui
                await applyEnhancement(ui)
            }
        }
        .task(id: enhanceKey) {
            if let ui = rawImage {
                await applyEnhancement(ui)
            }
        }
    }

    private func applyEnhancement(_ base: UIImage) async {
        if enhanceMode == .off && !autoCrop {
            displayImage = base
            showRevealAnimation = false
            return
        }
        if let hit = MetalImageEnhancer.shared.cachedResult(
            cacheKey: source.cacheKey,
            mode: enhanceMode,
            strength: enhanceStrength,
            autoCrop: autoCrop
        ) {
            displayImage = hit
            return
        }
        try? await Task.sleep(for: .milliseconds(70))
        guard !Task.isCancelled else { return }
        let key = source.cacheKey
        let m = enhanceMode
        let st = enhanceStrength
        let c = autoCrop
        let processed = await Task.detached(priority: .utility) {
            MetalImageEnhancer.shared.process(image: base, cacheKey: key, mode: m, strength: st, autoCrop: c)
        }.value
        guard !Task.isCancelled else { return }
        withAnimation(.easeInOut(duration: 0.30)) {
            displayImage = processed
            showRevealAnimation = true
        }
        try? await Task.sleep(for: .milliseconds(1100))
        if !Task.isCancelled {
            withAnimation(.easeOut(duration: 0.3)) {
                showRevealAnimation = false
            }
        }
    }
}

// MARK: - Archive Entry Wrapper (Local Comics & Archive Galleries)

struct ComicReaderView: View {
    let target: ComicArchiveTarget
    var kind: ReaderKind = .comic

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var pageSources: [ReaderPageSource] = []
    @State private var loading = true
    @State private var errorText = ""

    var body: some View {
        Group {
            if loading {
                ZStack {
                    Color.black.ignoresSafeArea()
                    ProgressView("解析页面中…")
                        .tint(.white)
                        .foregroundStyle(.white)
                }
            } else if !errorText.isEmpty {
                NavigationStack {
                    ContentUnavailableView("无法打开", systemImage: "exclamationmark.triangle", description: Text(errorText))
                        .toolbar {
                            ToolbarItem(placement: .topBarTrailing) { Button("关闭") { dismiss() } }
                        }
                }
            } else {
                ComicGalleryReaderView(kind: kind, title: target.title, sources: pageSources)
            }
        }
        .task { await load() }
    }

    private func load() async {
        guard let client = model.apiClient else {
            errorText = "未连接服务端"
            loading = false
            return
        }
        if let manifest = try? await client.comicArchiveManifest(assetId: target.id), !manifest.pages.isEmpty {
            ComicDirectStore.shared.remember(assetId: target.id, manifest)
            pageSources = manifest.pages.compactMap { page in
                guard let fallback = client.comicPageURL(assetId: target.id, pageIndex: page.index, thumb: false) else {
                    return nil
                }
                return .comicArchivePage(assetId: target.id, pageIndex: page.index, fallbackURL: fallback)
            }
            if !pageSources.isEmpty {
                loading = false
                return
            }
        }
        do {
            let info = try await client.inspectComicArchive(assetId: target.id)
            pageSources = info.pages.compactMap { page in
                guard let url = client.comicPageURL(assetId: target.id, pageIndex: page.index, thumb: false) else {
                    return nil
                }
                return .url(url)
            }
            if pageSources.isEmpty { errorText = "压缩包内没有可阅读图片" }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }
}
