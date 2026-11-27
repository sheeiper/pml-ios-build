import SwiftUI
import Metal
import UIKit

struct ComicArchiveTarget: Identifiable {
    let id: String
    let title: String
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

    inline float luma(float3 c) {
        return dot(c, float3(0.299, 0.587, 0.114));
    }

    kernel void enhance_anime(
        texture2d<float, access::sample> inTex [[texture(0)]],
        texture2d<float, access::write> outTex [[texture(1)]],
        uint2 gid [[thread_position_in_grid]]
    ) {
        uint w = outTex.get_width();
        uint h = outTex.get_height();
        if (gid.x >= w || gid.y >= h) return;

        constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::linear);
        float2 uv = (float2(gid) + 0.5) / float2(w, h);
        float2 texel = 1.0 / float2(inTex.get_width(), inTex.get_height());

        float3 c  = inTex.sample(s, uv).rgb;
        float3 n  = inTex.sample(s, uv + float2(0.0, -texel.y)).rgb;
        float3 so = inTex.sample(s, uv + float2(0.0,  texel.y)).rgb;
        float3 we = inTex.sample(s, uv + float2(-texel.x, 0.0)).rgb;
        float3 ea = inTex.sample(s, uv + float2( texel.x, 0.0)).rgb;
        float3 nw = inTex.sample(s, uv + float2(-texel.x, -texel.y)).rgb;
        float3 ne = inTex.sample(s, uv + float2( texel.x, -texel.y)).rgb;
        float3 sw = inTex.sample(s, uv + float2(-texel.x,  texel.y)).rgb;
        float3 se = inTex.sample(s, uv + float2( texel.x,  texel.y)).rgb;

        float lc = luma(c);
        float gx = (luma(ne) + 2.0 * luma(ea) + luma(se)) - (luma(nw) + 2.0 * luma(we) + luma(sw));
        float gy = (luma(sw) + 2.0 * luma(so) + luma(se)) - (luma(nw) + 2.0 * luma(n)  + luma(ne));
        float grad = clamp(sqrt(gx * gx + gy * gy), 0.0, 1.0);

        float3 avgFlat = (c * 4.0 + (n + so + we + ea) * 2.0 + (nw + ne + sw + se)) / 16.0;
        float flatW = smoothstep(0.16, 0.02, grad) * 0.42;
        float3 denoised = mix(c, avgFlat, flatW);

        float3 minNb = min(min(min(n, so), min(we, ea)), c);
        float3 maxNb = max(max(max(n, so), max(we, ea)), c);
        float3 blur = (n + so + we + ea) * 0.25;
        float3 detail = denoised - blur;

        float sharpStrength = mix(0.55, 1.45, smoothstep(0.05, 0.35, grad));
        float3 sharpened = denoised + detail * sharpStrength;

        float inkMask = smoothstep(0.55, 0.15, lc) * smoothstep(0.08, 0.30, grad);
        sharpened = mix(sharpened, min(sharpened, minNb * 0.96), inkMask * 0.38);

        float3 finalCol = clamp(sharpened, minNb - float3(0.06), maxNb + float3(0.06));
        outTex.write(float4(clamp(finalCol, 0.0, 1.0), 1.0), gid);
    }

    kernel void enhance_photo(
        texture2d<float, access::sample> inTex [[texture(0)]],
        texture2d<float, access::write> outTex [[texture(1)]],
        uint2 gid [[thread_position_in_grid]]
    ) {
        uint w = outTex.get_width();
        uint h = outTex.get_height();
        if (gid.x >= w || gid.y >= h) return;

        constexpr sampler s(coord::normalized, address::clamp_to_edge, filter::linear);
        float2 uv = (float2(gid) + 0.5) / float2(w, h);
        float2 texel = 1.0 / float2(inTex.get_width(), inTex.get_height());

        float3 a = inTex.sample(s, uv + float2(-texel.x, -texel.y)).rgb;
        float3 b = inTex.sample(s, uv + float2( 0.0,     -texel.y)).rgb;
        float3 c = inTex.sample(s, uv + float2( texel.x, -texel.y)).rgb;
        float3 d = inTex.sample(s, uv + float2(-texel.x,  0.0)).rgb;
        float3 e = inTex.sample(s, uv).rgb;
        float3 f = inTex.sample(s, uv + float2( texel.x,  0.0)).rgb;
        float3 g = inTex.sample(s, uv + float2(-texel.x,  texel.y)).rgb;
        float3 h = inTex.sample(s, uv + float2( 0.0,      texel.y)).rgb;
        float3 i = inTex.sample(s, uv + float2( texel.x,  texel.y)).rgb;

        float3 mnRGB = min(min(min(d, e), min(f, b)), h);
        mnRGB += min(mnRGB, min(min(a, c), min(g, i)));
        float3 mxRGB = max(max(max(d, e), max(f, b)), h);
        mxRGB += max(mxRGB, max(max(a, c), max(g, i)));

        float3 rcpMRGB = 1.0 / max(mxRGB, float3(0.0001));
        float3 ampRGB = sqrt(clamp(min(mnRGB, 2.0 - mxRGB) * rcpMRGB, 0.0, 1.0));
        float peak = -1.0 / mix(8.0, 5.0, 0.72);
        float3 wRGB = ampRGB * peak;
        float3 outRGB = clamp(((b + d + f + h) * wRGB + e) / (1.0 + 4.0 * wRGB), 0.0, 1.0);
        float3 clarity = outRGB * outRGB * (3.0 - 2.0 * outRGB);
        outRGB = mix(outRGB, clarity, 0.08);
        outTex.write(float4(outRGB, 1.0), gid);
    }
    """

    private init() {
        cache.countLimit = 24
        guard let dev = MTLCreateSystemDefaultDevice(),
              let q = dev.makeCommandQueue(),
              let lib = try? dev.makeLibrary(source: Self.shaderSource, options: nil) else {
            self.device = nil
            self.queue = nil
            self.animePipeline = nil
            self.photoPipeline = nil
            return
        }
        self.device = dev
        self.queue = q
        self.animePipeline = lib.makeFunction(name: "enhance_anime").flatMap { try? dev.makeComputePipelineState(function: $0) }
        self.photoPipeline = lib.makeFunction(name: "enhance_photo").flatMap { try? dev.makeComputePipelineState(function: $0) }
    }

    func process(image: UIImage, cacheKey: String, mode: ReaderEnhanceMode, autoCrop: Bool) -> UIImage {
        if mode == .off && !autoCrop { return image }
        let fullKey = "\(mode.rawValue)|\(autoCrop ? 1 : 0)|\(cacheKey)" as NSString
        if let hit = cache.object(forKey: fullKey) { return hit }

        var working = image
        if autoCrop, let cropped = detectAndCrop(working) {
            working = cropped
        }
        if mode != .off, let enhanced = runMetal(working, mode: mode) {
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

    private func runMetal(_ image: UIImage, mode: ReaderEnhanceMode) -> UIImage? {
        guard let dev = device,
              let queue = queue,
              let pipeline = (mode == .anime ? animePipeline : photoPipeline),
              let cg = image.cgImage else { return nil }

        let srcW = cg.width
        let srcH = cg.height
        let maxDim = 2200
        let scale: Double = (mode == .anime && max(srcW, srcH) < 1500)
            ? min(1.5, Double(maxDim) / Double(max(srcW, srcH)))
            : min(1.0, Double(maxDim) / Double(max(srcW, srcH)))
        let outW = max(1, Int(Double(srcW) * scale))
        let outH = max(1, Int(Double(srcH) * scale))

        // Upload RGBA8 input texture
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

        enc.setComputePipelineState(pipeline)
        enc.setTexture(inTex, index: 0)
        enc.setTexture(outTex, index: 1)
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

struct ComicGalleryReaderView: View {
    let kind: ReaderKind
    let title: String
    let pageURLs: [URL]
    var onDismiss: (() -> Void)? = nil

    @Environment(\.dismiss) private var dismiss

    @State private var readMode: ReaderLayoutMode = .strip
    @State private var enhanceMode: ReaderEnhanceMode = .off
    @State private var autoCrop: Bool = false
    @State private var warmBg: Bool = false

    @State private var currentIndex: Int = 0
    @State private var sliderValue: Double = 1
    @State private var chromeVisible: Bool = false
    @State private var activePopover: PopoverKind? = nil
    @State private var peekingOriginal: Bool = false
    @State private var zoomScale: CGFloat = 1.0

    private enum PopoverKind { case mode, enhance, display }

    private var prefsPrefix: String { "pml.ios.reader.\(kind.rawValue)." }

    var body: some View {
        ZStack {
            (warmBg ? Color(red: 0.96, green: 0.95, blue: 0.92) : Color.black)
                .ignoresSafeArea()

            if pageURLs.isEmpty {
                ProgressView("加载页面中…")
                    .tint(.white)
                    .foregroundStyle(.secondary)
            } else if readMode == .strip {
                stripCanvas
            } else {
                pagedCanvas
            }

            // 极淡角落页码（沉浸态）
            if !chromeVisible && !pageURLs.isEmpty {
                VStack {
                    Spacer()
                    HStack {
                        Spacer()
                        Text("\(currentIndex + 1) / \(pageURLs.count)")
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
                    Text("原图对比")
                        .font(.caption.bold())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 6)
                        .background(.ultraThinMaterial, in: Capsule())
                        .padding(.top, 14)
                    Spacer()
                }
                .allowsHitTesting(false)
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
    }

    private var stripCanvas: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(pageURLs.enumerated()), id: \.offset) { idx, url in
                        EnhancedReaderPageImage(
                            url: url,
                            enhanceMode: peekingOriginal ? .off : enhanceMode,
                            autoCrop: peekingOriginal ? false : autoCrop
                        )
                        .id(idx)
                        .onAppear {
                            currentIndex = idx
                            sliderValue = Double(idx + 1)
                        }
                    }
                }
                .scaleEffect(zoomScale, anchor: .top)
            }
            .onTapGesture(count: 2) {
                withAnimation(.easeInOut(duration: 0.2)) {
                    zoomScale = zoomScale > 1.1 ? 1.0 : 2.0
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
            .onLongPressGesture(minimumDuration: 0.25, pressing: { pressing in
                if enhanceMode != .off || autoCrop {
                    peekingOriginal = pressing
                }
            }, perform: {})
            .onChange(of: sliderValue) { _, newVal in
                let target = max(0, min(pageURLs.count - 1, Int(newVal) - 1))
                if abs(target - currentIndex) > 1 {
                    proxy.scrollTo(target, anchor: .top)
                }
            }
        }
    }

    private var pagedCanvas: some View {
        TabView(selection: $currentIndex) {
            ForEach(Array(pageURLs.enumerated()), id: \.offset) { idx, url in
                EnhancedReaderPageImage(
                    url: url,
                    enhanceMode: peekingOriginal ? .off : enhanceMode,
                    autoCrop: peekingOriginal ? false : autoCrop
                )
                .scaleEffect(zoomScale)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .tag(idx)
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        .environment(\.layoutDirection, readMode == .manga ? .rightToLeft : .leftToRight)
        .onChange(of: currentIndex) { _, newVal in
            sliderValue = Double(newVal + 1)
            zoomScale = 1.0
        }
        .onTapGesture(count: 2) {
            withAnimation(.easeInOut(duration: 0.2)) {
                zoomScale = zoomScale > 1.1 ? 1.0 : 2.1
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
        .onLongPressGesture(minimumDuration: 0.25, pressing: { pressing in
            if enhanceMode != .off || autoCrop {
                peekingOriginal = pressing
            }
        }, perform: {})
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
            Color.clear.frame(width: 58, height: 1)
        }
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.ultraThinMaterial)
    }

    private var bottomBar: some View {
        VStack(spacing: 10) {
            if pageURLs.count > 1 {
                HStack(spacing: 10) {
                    Text("\(Int(sliderValue))")
                        .font(.caption.monospacedDigit())
                        .frame(minWidth: 30)
                    Slider(
                        value: $sliderValue,
                        in: 1...Double(max(1, pageURLs.count)),
                        step: 1
                    ) { editing in
                        if !editing {
                            currentIndex = max(0, min(pageURLs.count - 1, Int(sliderValue) - 1))
                        }
                    }
                    .tint(.cyan)
                    Text("\(pageURLs.count)")
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
                    Text("长按画面对比原图").font(.caption2).foregroundStyle(.cyan)
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
            }
        }
        .padding(14)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 18))
        .overlay(RoundedRectangle(cornerRadius: 18).stroke(Color.white.opacity(0.14), lineWidth: 1))
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
        autoCrop = defaults.bool(forKey: prefsPrefix + "crop")
        warmBg = defaults.bool(forKey: prefsPrefix + "warm")
    }

    private func savePrefs() {
        let defaults = UserDefaults.standard
        defaults.set(readMode.rawValue, forKey: prefsPrefix + "mode")
        defaults.set(enhanceMode.rawValue, forKey: prefsPrefix + "enhance")
        defaults.set(autoCrop, forKey: prefsPrefix + "crop")
        defaults.set(warmBg, forKey: prefsPrefix + "warm")
    }
}

private struct EnhancedReaderPageImage: View {
    let url: URL
    let enhanceMode: ReaderEnhanceMode
    let autoCrop: Bool

    @State private var rawImage: UIImage?
    @State private var displayImage: UIImage?

    var body: some View {
        Group {
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
        }
        .task(id: url) {
            if let ui = await ImageCache.shared.image(for: url) {
                rawImage = ui
                await applyEnhancement(ui)
            }
        }
        .task(id: "\(enhanceMode.rawValue)-\(autoCrop)") {
            if let ui = rawImage {
                await applyEnhancement(ui)
            }
        }
    }

    private func applyEnhancement(_ base: UIImage) async {
        if enhanceMode == .off && !autoCrop {
            displayImage = base
            return
        }
        let key = url.absoluteString
        let m = enhanceMode
        let c = autoCrop
        let processed = await Task.detached(priority: .userInitiated) {
            MetalImageEnhancer.shared.process(image: base, cacheKey: key, mode: m, autoCrop: c)
        }.value
        withAnimation(.easeInOut(duration: 0.22)) {
            displayImage = processed
        }
    }
}

// MARK: - Archive Entry Wrapper (Local Comics & Archive Galleries)

struct ComicReaderView: View {
    let target: ComicArchiveTarget
    var kind: ReaderKind = .comic

    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var pageURLs: [URL] = []
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
                ComicGalleryReaderView(kind: kind, title: target.title, pageURLs: pageURLs)
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
        do {
            let info = try await client.inspectComicArchive(assetId: target.id)
            pageURLs = info.pages.compactMap { client.comicPageURL(assetId: target.id, pageIndex: $0.index, thumb: false) }
            if pageURLs.isEmpty { errorText = "压缩包内没有可阅读图片" }
        } catch {
            errorText = error.localizedDescription
        }
        loading = false
    }
}
