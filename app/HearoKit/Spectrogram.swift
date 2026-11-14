import Accelerate
import CoreGraphics
import Foundation

/// Short-time Fourier transform of a mono clip, for the review window's spectrogram lane.
///
/// Pure value type: no UI, no actors, testable. Frame `i` is centred on sample
/// `start + i * hop` (zero-padded past the clip edges), Hann-windowed, and stored as power in
/// dB re a full-scale sine (a 0 dBFS sine reads ≈ 0 dB in its bin). Bins run 0 … fftSize/2
/// (DC … Nyquist).
public struct Spectrogram: Sendable, Equatable {
    public let sampleRate: Double
    public let fftSize: Int
    public let hop: Int
    /// Sample index the first frame is centred on.
    public let start: Int
    public let frames: Int
    /// `fftSize / 2 + 1`.
    public let bins: Int
    /// dB, frame-major: `db[frame * bins + bin]`.
    public let db: [Float]
    /// Loudest bin anywhere (the colour map's top).
    public let maxDb: Float

    /// Floor for exact silence (−200 dB, far below any display range).
    public static let silenceDb: Float = -200

    public var binHz: Double { sampleRate / Double(fftSize) }
    public func frequency(ofBin k: Int) -> Double { Double(k) * binHz }
    /// Centre time (s) of a frame.
    public func time(ofFrame i: Int) -> Double { Double(start + i * hop) / sampleRate }
    public func value(frame: Int, bin: Int) -> Float { db[frame * bins + bin] }

    /// Time span (s) the frames cover when each is drawn `hop` wide around its centre.
    public var extent: ClosedRange<Double> {
        (Double(start) - Double(hop) / 2) / sampleRate...(Double(start) + (Double(frames) - 0.5) * Double(hop)) / sampleRate
    }

    /// The whole clip: frames centred on 0, hop, 2·hop, … up to the last sample.
    public static func compute(
        samples: [Float], sampleRate: Double, fftSize: Int = 1024, hop: Int = 128, isCancelled: () -> Bool = { false }
    ) -> Spectrogram? {
        let frames = samples.isEmpty ? 0 : (samples.count - 1) / max(hop, 1) + 1
        return compute(
            samples: samples, sampleRate: sampleRate, fftSize: fftSize, hop: hop, start: 0, frames: frames,
            isCancelled: isCancelled)
    }

    /// `frames` frames centred on `start`, `start + hop`, …; `nil` when cancelled.
    /// `fftSize` must be a power of two ≥ 16.
    public static func compute(
        samples: [Float], sampleRate: Double, fftSize: Int, hop: Int, start: Int, frames: Int,
        isCancelled: () -> Bool = { false }
    ) -> Spectrogram? {
        precondition(fftSize >= 16 && fftSize & (fftSize - 1) == 0, "fftSize must be a power of two")
        let hop = max(1, hop)
        let frames = max(0, frames)
        let half = fftSize / 2
        let bins = half + 1
        let log2n = vDSP_Length(fftSize.trailingZeroBitCount)
        guard let setup = vDSP_create_fftsetup(log2n, FFTRadix(kFFTRadix2)) else { return nil }
        defer { vDSP_destroy_fftsetup(setup) }

        let window = vDSP.window(ofType: Float.self, usingSequence: .hanningDenormalized, count: fftSize, isHalfWindow: false)
        // A sine of amplitude A peaks at A·N/4 through a Hann window, ×2 from zrip: A·N/2.
        // Scale power by (2/N)² so a full-scale sine reads 0 dB.
        let scale = Float(4.0 / (Double(fftSize) * Double(fftSize)))
        var out = [Float](repeating: silenceDb, count: frames * bins)
        var frame = [Float](repeating: 0, count: fftSize)
        var re = [Float](repeating: 0, count: half)
        var im = [Float](repeating: 0, count: half)
        var power = [Float](repeating: 0, count: bins)
        let n = samples.count
        var cancelled = false

        samples.withUnsafeBufferPointer { src in
          window.withUnsafeBufferPointer { win in
            re.withUnsafeMutableBufferPointer { reP in
                im.withUnsafeMutableBufferPointer { imP in
                    var split = DSPSplitComplex(realp: reP.baseAddress!, imagp: imP.baseAddress!)
                    for f in 0..<frames {
                        if f & 63 == 0, isCancelled() {
                            cancelled = true
                            return
                        }
                        // Gather the windowed frame (zero outside the clip).
                        let w0 = start + f * hop - half
                        let a = max(0, w0)
                        let b = min(n, w0 + fftSize)
                        frame.withUnsafeMutableBufferPointer { fr in
                            vDSP.fill(&fr, with: 0)
                            if b > a, let base = src.baseAddress {
                                vDSP_vmul(base + a, 1, win.baseAddress! + (a - w0), 1, fr.baseAddress! + (a - w0), 1, vDSP_Length(b - a))
                            }
                            fr.baseAddress!.withMemoryRebound(to: DSPComplex.self, capacity: half) {
                                vDSP_ctoz($0, 2, &split, 1, vDSP_Length(half))
                            }
                        }
                        vDSP_fft_zrip(setup, &split, 1, log2n, FFTDirection(FFT_FORWARD))
                        power.withUnsafeMutableBufferPointer { p in
                            let pb = p.baseAddress!
                            vDSP_zvmags(&split, 1, pb, 1, vDSP_Length(half))
                            // zrip packs DC in realp[0] and Nyquist in imagp[0], both at twice the
                            // scale of the other bins (real-valued, no mirror image): quarter power.
                            let dc = reP[0]
                            let ny = imP[0]
                            pb[0] = dc * dc / 4
                            pb[half] = ny * ny / 4
                            var s = scale
                            vDSP_vsmul(pb, 1, &s, pb, 1, vDSP_Length(bins))
                            var eps: Float = 1e-20
                            vDSP_vsadd(pb, 1, &eps, pb, 1, vDSP_Length(bins))
                            out.withUnsafeMutableBufferPointer { o in
                                var ref: Float = 1
                                // 10·log10(p / 1): power dB.
                                vDSP_vdbcon(pb, 1, &ref, o.baseAddress! + f * bins, 1, vDSP_Length(bins), 0)
                            }
                        }
                    }
                }
            }
          }
        }
        if cancelled { return nil }
        var mx = silenceDb
        if !out.isEmpty { mx = max(silenceDb, vDSP.maximum(out)) }
        return Spectrogram(
            sampleRate: sampleRate, fftSize: fftSize, hop: hop, start: start, frames: frames, bins: bins, db: out, maxDb: mx)
    }

    /// Which bins feed each display row (row 0 = top = highest frequency). Wide rows (high
    /// frequencies, ≥ 2 bins) take their loudest bin so narrow lines aren't lost; the narrow rows
    /// below them interpolate linearly between the two nearest bins (low frequencies on a log
    /// axis). Rows get narrower going down, so wide rows are `0..<bandRows` and the rest follow.
    struct RowMap {
        var lo: [Int] = []
        var len: [Int] = []
        /// Fractional bin index of each interpolated row (`rows - bandRows` entries).
        var position: [Float] = []
        var bandRows: Int { lo.count }

        init(scale: FrequencyScale, rows: Int, binHz: Double, bins: Int) {
            for r in 0..<rows {
                let top = scale.hz(atRow: Double(r), rows: rows) / binHz
                let bottom = scale.hz(atRow: Double(r + 1), rows: rows) / binHz
                if position.isEmpty, top - bottom >= 2 {
                    let b0 = min(max(0, Int(bottom.rounded(.up))), bins - 1)
                    let b1 = min(max(b0, Int(top.rounded(.down))), bins - 1)
                    lo.append(b0)
                    len.append(b1 - b0 + 1)
                } else {
                    let centre = scale.hz(atRow: Double(r) + 0.5, rows: rows) / binHz
                    position.append(Float(min(max(0, centre), Double(bins) - 1.001)))
                }
            }
        }
    }

    /// Display values, row-major (`rows` × `frames`, top row first); `nil` when cancelled.
    func columns(_ map: RowMap, rows: Int, isCancelled: () -> Bool = { false }) -> [Float]? {
        // Frame-major first (each frame's rows contiguous), then one transpose.
        var byFrame = [Float](repeating: 0, count: rows * frames)
        var cancelled = false
        db.withUnsafeBufferPointer { d in
            map.lo.withUnsafeBufferPointer { lo in
                map.len.withUnsafeBufferPointer { len in
                    map.position.withUnsafeBufferPointer { pos in
                        byFrame.withUnsafeMutableBufferPointer { out in
                            for f in 0..<frames {
                                if f & 255 == 0, isCancelled() {
                                    cancelled = true
                                    return
                                }
                                let src = d.baseAddress! + f * bins
                                let dst = out.baseAddress! + f * rows
                                for r in 0..<lo.count {
                                    vDSP_maxv(src + lo[r], 1, dst + r, vDSP_Length(len[r]))
                                }
                                if let p = pos.baseAddress {
                                    vDSP_vlint(src, p, 1, dst + lo.count, 1, vDSP_Length(pos.count), vDSP_Length(bins))
                                }
                            }
                        }
                    }
                }
            }
        }
        if cancelled { return nil }
        var grid = [Float](repeating: 0, count: rows * frames)
        vDSP_mtrans(byFrame, 1, &grid, 1, vDSP_Length(rows), vDSP_Length(frames))
        return grid
    }

    /// One display value per (row, frame), row-major, top row first.
    public func grid(scale: FrequencyScale, rows: Int) -> [Float] {
        columns(RowMap(scale: scale, rows: rows, binHz: binHz, bins: bins), rows: rows) ?? []
    }

    /// Render to an RGB image: one pixel column per frame, `rows` pixel rows (top = high),
    /// colour-mapped from `top − rangeDb` (floor, darkest) to `top` (brightest). `top` defaults
    /// to the loudest bin but never below −120 dB, so a silent clip renders as the floor.
    public func image(
        scale: FrequencyScale, rows: Int, top: Float? = nil, rangeDb: Float = 80, isCancelled: () -> Bool = { false }
    ) -> CGImage? {
        guard frames > 0, rows > 0,
            let grid = columns(RowMap(scale: scale, rows: rows, binHz: binHz, bins: bins), rows: rows, isCancelled: isCancelled)
        else { return nil }
        let hi = max(top ?? maxDb, -120)
        let lo = hi - rangeDb
        // dB → colour index 0…255 (vectorised), then index → RGBX through a 32-bit table.
        let count = rows * frames
        var index = [Float](repeating: 0, count: count)
        var a = 255 / rangeDb
        var b = -lo * 255 / rangeDb
        var zero: Float = 0
        var ceiling: Float = 255
        vDSP_vsmsa(grid, 1, &a, &b, &index, 1, vDSP_Length(count))
        vDSP_vclip(index, 1, &zero, &ceiling, &index, 1, vDSP_Length(count))
        var bytes = [UInt8](repeating: 0, count: count)
        vDSP_vfixu8(index, 1, &bytes, 1, vDSP_Length(count))
        guard !isCancelled() else { return nil }
        var pixels = [UInt32](repeating: 0, count: count)
        Colormap.magmaRGBX.withUnsafeBufferPointer { lut in
            bytes.withUnsafeBufferPointer { src in
                pixels.withUnsafeMutableBufferPointer { dst in
                    let l = lut.baseAddress!
                    let s = src.baseAddress!
                    let d = dst.baseAddress!
                    for i in 0..<count { d[i] = l[Int(s[i])] }
                }
            }
        }
        let data = pixels.withUnsafeBytes { Data($0) }
        guard let provider = CGDataProvider(data: data as CFData) else { return nil }
        return CGImage(
            width: frames, height: rows, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: frames * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue), provider: provider, decode: nil,
            shouldInterpolate: true, intent: .defaultIntent)
    }
}

/// Vertical axis of the spectrogram: log frequency between `minHz` and `maxHz` (speech: hum at
/// 50/60 Hz and sibilance at 5–10 kHz both get room). Row 0 is the top edge (`maxHz`).
public struct FrequencyScale: Sendable, Equatable, Hashable {
    public let minHz: Double
    public let maxHz: Double

    public init(minHz: Double = 30, maxHz: Double = 20_000) {
        self.minHz = max(1, minHz)
        self.maxHz = max(self.minHz * 2, maxHz)
    }

    /// The default for a clip: 30 Hz … min(20 kHz, Nyquist).
    public static func speech(sampleRate: Double) -> FrequencyScale {
        FrequencyScale(minHz: 30, maxHz: min(20_000, sampleRate / 2))
    }

    /// 0 at `minHz`, 1 at `maxHz` (unclamped).
    public func fraction(ofHz hz: Double) -> Double { log(max(hz, 1e-9) / minHz) / log(maxHz / minHz) }
    public func hz(atFraction f: Double) -> Double { minHz * pow(maxHz / minHz, f) }
    /// Continuous row position (0 = top edge, `rows` = bottom edge).
    public func row(ofHz hz: Double, rows: Int) -> Double { (1 - fraction(ofHz: hz)) * Double(rows) }
    public func hz(atRow row: Double, rows: Int) -> Double { hz(atFraction: 1 - row / Double(max(rows, 1))) }

    /// Labelled ticks that fit inside the range (100, 1k, 5k, 10k first; then 50, 500, 2k, 20k).
    public var ticks: [Double] {
        [50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000].filter { $0 >= minHz && $0 <= maxHz }
    }

    public static func label(_ hz: Double) -> String {
        hz >= 1000 ? "\(Int(hz / 1000))k" : "\(Int(hz))"
    }
}

/// How the STFT trades time against frequency detail. Time detail matters for smacks and clicks
/// (2–20 ms); frequency detail for hum (50/60 Hz and harmonics).
public enum SpectrogramResolution: String, CaseIterable, Identifiable, Sendable {
    case time
    case balanced
    case frequency

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .time: "Time detail"
        case .balanced: "Balanced"
        case .frequency: "Frequency detail"
        }
    }

    /// Target window length (s); the FFT size is the power of two nearest to it.
    var windowSeconds: Double {
        switch self {
        case .time: 0.010
        case .balanced: 0.021
        case .frequency: 0.085
        }
    }

    /// FFT size at `sampleRate` (48 kHz: 512 / 1024 / 4096).
    public func fftSize(sampleRate: Double) -> Int {
        let n = max(16, sampleRate * windowSeconds)
        return 1 << Int(log2(n).rounded())
    }

    /// Overview hop: 1/8 of the window (48 kHz: 1.3 / 2.7 / 10.7 ms).
    public func hop(sampleRate: Double) -> Int { fftSize(sampleRate: sampleRate) / 8 }
}

/// Perceptually uniform colour maps as 256-entry RGB lookup tables (3 bytes per entry).
public enum Colormap {
    /// Matplotlib's magma (dark purple-black → orange → pale yellow), sampled at 11 stops and
    /// interpolated linearly; uniform enough for reading levels, readable in light and dark mode.
    public static let magma: [UInt8] = lut([
        (0.001, 0.000, 0.014), (0.079, 0.054, 0.212), (0.232, 0.060, 0.438), (0.390, 0.100, 0.502),
        (0.550, 0.161, 0.506), (0.716, 0.215, 0.475), (0.869, 0.288, 0.409), (0.968, 0.440, 0.360),
        (0.995, 0.624, 0.427), (0.995, 0.813, 0.573), (0.987, 0.991, 0.750),
    ])

    /// `magma` as 32-bit pixels whose bytes in memory are R, G, B, 0xFF (for RGBX images).
    public static let magmaRGBX: [UInt32] = (0..<256).map { i in
        let r = UInt32(magma[i * 3])
        let g = UInt32(magma[i * 3 + 1])
        let b = UInt32(magma[i * 3 + 2])
        return UInt32(littleEndian: r | g << 8 | b << 16 | 0xFF << 24)
    }

    static func lut(_ stops: [(Double, Double, Double)]) -> [UInt8] {
        var out = [UInt8]()
        out.reserveCapacity(256 * 3)
        for i in 0..<256 {
            let x = Double(i) / 255 * Double(stops.count - 1)
            let k = min(Int(x), stops.count - 2)
            let t = x - Double(k)
            let a = stops[k]
            let b = stops[k + 1]
            for (u, v) in [(a.0, b.0), (a.1, b.1), (a.2, b.2)] {
                out.append(UInt8((u + (v - u) * t) * 255 + 0.5))
            }
        }
        return out
    }
}
