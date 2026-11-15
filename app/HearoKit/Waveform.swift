import Foundation
import Observation

/// New waveform columns from the engine (60 per second, oldest first), aligned with the output.
public struct WaveformChunk: Sendable, Equatable {
    public var next: UInt64
    /// Original (unprocessed) peak per column, 0…1.
    public var peaks: [Float]
    /// Some cleanup module cut more than 1 dB in the column.
    public var removed: [Bool]
    /// Processed output peak per column, 0…1 (empty from older producers → use `peaks`).
    public var outputPeaks: [Float]
    /// Gain reduction per column for each cleanup module, chain order (`cleanupModuleKeys`), in
    /// dB ≥ 0 (0.25 dB steps). Draw outside-in: the outer part of a column's height in
    /// proportion to each module's reduction.
    public var reductions: [ModuleReduction]

    public init(
        next: UInt64, peaks: [Float] = [], removed: [Bool] = [], outputPeaks: [Float] = [],
        reductions: [ModuleReduction] = []
    ) {
        self.next = next
        self.peaks = peaks
        self.removed = removed
        self.outputPeaks = outputPeaks
        self.reductions = reductions
    }
}

/// The live waveform's columns as an observable of their own: views that read `buffer` redraw
/// on every poll; views that only hold the `LiveWaveform` don't.
@MainActor
@Observable
public final class LiveWaveform {
    public internal(set) var buffer: WaveformBuffer
    public init(capacity: Int) { buffer = WaveformBuffer(capacity: capacity) }
}

/// The last `capacity` columns shown in the popover (newest last = right edge).
public struct WaveformBuffer: Sendable, Equatable {
    public struct Column: Sendable, Equatable {
        public var peak: Float
        public var removed: Bool
        /// Processed peak (equals `peak` when the producer didn't send one).
        public var outPeak: Float
        /// dB per cleanup module, `cleanupModuleKeys` order (empty when not sent).
        public var reductionDb: [Float]

        public init(peak: Float, removed: Bool, outPeak: Float? = nil, reductionDb: [Float] = []) {
            self.peak = peak
            self.removed = removed
            self.outPeak = outPeak ?? peak
            self.reductionDb = reductionDb
        }
    }

    public let capacity: Int
    public private(set) var columns: [Column] = []

    public init(capacity: Int) { self.capacity = capacity }

    public mutating func append(_ chunk: WaveformChunk) {
        // Rows by key, so index k is always `cleanupModuleKeys[k]` (its colour) whatever order
        // or subset the producer sends.
        let rows = chunk.reductions.isEmpty
            ? [] : cleanupModuleKeys.map { key in chunk.reductions.first(where: { $0.key == key })?.gainReductionDb ?? [] }
        for (i, (p, r)) in zip(chunk.peaks, chunk.removed).enumerated() {
            let out = i < chunk.outputPeaks.count ? chunk.outputPeaks[i] : nil
            let gr = rows.map { i < $0.count ? $0[i] : 0 }
            columns.append(Column(peak: p, removed: r, outPeak: out, reductionDb: gr))
        }
        if columns.count > capacity { columns.removeFirst(columns.count - capacity) }
    }

    public mutating func clear() { columns.removeAll() }

    /// Bar height 0…1 for a peak: dBFS mapped linearly from `floorDb` (0) to 0 dBFS (1), so quiet
    /// speech is still visible and a smack stands out.
    public static func height(_ peak: Float, floorDb: Float = -54) -> Float {
        guard peak > 0 else { return 0 }
        let db = 20 * log10(peak)
        return min(1, max(0, (db - floorDb) / -floorDb))
    }
}

/// How one waveform column is coloured, outside-in: the stroke's full height is the original
/// peak; the uncoloured **core** is what remains after processing; the rim between them is split
/// among the cleanup modules in proportion to each module's dB reduction, in chain order from the
/// outside in (`cleanupModuleKeys`: de-hum outermost … de-ess innermost — the first module in the
/// chain removed the outermost part; a fixed order also keeps each colour in a predictable place).
///
/// Fractions are linear amplitude, so the coloured share of the stroke is the share of the
/// amplitude removed: −12 dB colours 75 % of the stroke, −2 dB 21 %, −0.5 dB 6 %. The core is
/// the measured output peak when there is one, but never smaller than the modules' reported
/// reduction allows (`10^(−Σr/20)`), so limiter gain or a peak elsewhere in the column is never
/// painted as a cleanup module. Reductions under `minDb` (meter noise) are ignored.
public struct ReductionBands: Sendable, Equatable {
    public struct Band: Sendable, Equatable {
        /// Index into `cleanupModuleKeys` (and the reductions row).
        public var module: Int
        /// Fractions of the stroke's half-height from the centre line; `inner < outer`.
        public var inner: Float
        public var outer: Float
        /// The module's reduction in this column.
        public var db: Float
        public var thickness: Float { outer - inner }
    }

    /// Uncoloured core, fraction of the stroke (0…1; 1 = nothing removed).
    public var core: Float
    /// Outside in: the first band's `outer` is 1, the last band's `inner` is `core`.
    public var bands: [Band]

    /// Smaller reductions are meter noise, not "removed something".
    public static let minDb: Float = 0.5
    public static let none = ReductionBands(core: 1, bands: [])

    /// `peak`: original peak; `outPeak`: processed peak (nil = not measured); `reductionsDb`:
    /// per cleanup module, chain order, dB ≥ 0.
    public static func compute(peak: Float, outPeak: Float?, reductionsDb: [Float]) -> ReductionBands {
        let r = reductionsDb.map { $0.isFinite && $0 >= minDb ? $0 : 0 }
        let total = r.reduce(0, +)
        guard peak > 0, peak.isFinite, total > 0 else { return .none }
        let claimed = pow(10, -total / 20)
        var core = claimed
        if let out = outPeak, out.isFinite { core = max(claimed, min(1, max(0, out / peak))) }
        let rim = 1 - core
        guard rim > 1e-6 else { return .none }
        var bands: [Band] = []
        var outer: Float = 1
        for (i, db) in r.enumerated() where db > 0 {
            let inner = max(core, outer - rim * db / total)
            bands.append(Band(module: i, inner: inner, outer: outer, db: db))
            outer = inner
        }
        if !bands.isEmpty { bands[bands.count - 1].inner = core }  // no rounding gap at the core
        return ReductionBands(core: core, bands: bands)
    }
}
