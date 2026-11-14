import CoreGraphics
import Foundation
import Observation

/// A colour-mapped spectrogram ready to draw: one pixel column per STFT frame.
public struct SpectrogramImage: @unchecked Sendable {  // CGImage is immutable and thread-safe
    public let image: CGImage
    /// Seconds covered by the columns (each frame drawn one hop wide around its centre).
    public let extent: ClosedRange<Double>
    public let frames: Int
    public let hop: Int
    public let sampleRate: Double
    public let scale: FrequencyScale
    /// dB mapped to the brightest colour (the clip's loudest bin).
    public let topDb: Float

    /// Seconds per column.
    public var columnSeconds: Double { Double(hop) / sampleRate }

    /// The columns overlapping `[start, end]`, and the time span they cover; `nil` if none.
    public func slice(from start: Double, to end: Double) -> (columns: Range<Int>, span: ClosedRange<Double>)? {
        let c0 = max(0, Int(((start - extent.lowerBound) / columnSeconds).rounded(.down)))
        let c1 = min(frames, Int(((end - extent.lowerBound) / columnSeconds).rounded(.up)))
        guard c1 > c0 else { return nil }
        let t0 = extent.lowerBound + Double(c0) * columnSeconds
        return (c0..<c1, t0...(t0 + Double(c1 - c0) * columnSeconds))
    }
}

/// A higher-time-resolution render of part of a clip, for deep zoom where the overview's
/// columns would be several pixels wide. Frames are centred on `start`, `start + hop`, ….
public struct SpectrogramDetail: Hashable, Sendable {
    public let start: Int
    public let frames: Int
    public let hop: Int

    /// Frames per alignment block: the detail only changes when the view leaves its blocks or
    /// the zoom crosses a power of two, so panning doesn't recompute.
    static let block = 256

    /// What to render for this view, or `nil` when the overview (hop `overviewHop`) already has
    /// at least one column per two pixels, or when the window (`fftSize`) smears time so much
    /// that a finer hop wouldn't look sharper (≥ fftSize/32 samples per pixel). `pixels` is the
    /// lane width in device pixels.
    public static func plan(
        viewport: Viewport, pixels: Double, sampleRate: Double, overviewHop: Int, fftSize: Int, clipSamples: Int
    ) -> SpectrogramDetail? {
        guard pixels >= 1, clipSamples > 0 else { return nil }
        let samplesPerPixel = viewport.length * sampleRate / pixels
        guard Double(overviewHop) > 2 * samplesPerPixel, samplesPerPixel < Double(fftSize) / 32 else { return nil }
        // Largest power of two ≤ samples per pixel: at least one column per pixel.
        let hop = 1 << max(0, Int(log2(max(1, samplesPerPixel)).rounded(.down)))
        let span = hop * block
        let first = max(0, Int((viewport.start * sampleRate / Double(span)).rounded(.down)) - 1)
        let last = min(
            (clipSamples - 1) / span + 1,
            Int((viewport.end * sampleRate / Double(span)).rounded(.up)) + 1)
        guard last > first else { return nil }
        return SpectrogramDetail(start: first * span, frames: (last - first) * block, hop: hop)
    }
}

/// Spectrogram images, computed off the main thread. Overviews (whole clip) are kept for the
/// last few clips/resolutions so switching back is instant; deep-zoom details live in a separate,
/// smaller store so zooming around never evicts the overview on screen. Views call `load` from
/// `.task(id:)`, so leaving a clip (or zooming elsewhere) cancels its computation.
@MainActor
@Observable
public final class SpectrogramCache {
    /// Pixel rows (log frequency) of every image; the lane scales them to its height.
    public nonisolated static let rows = 320
    /// Overviews kept (~3–8 MB each for an 8 s clip).
    static let overviewCapacity = 6
    /// Details kept (the current one and its predecessor, drawn while the next computes).
    static let detailCapacity = 3

    public struct Key: Hashable, Sendable {
        public var clip: String
        public var resolution: SpectrogramResolution
        /// `nil` = the whole clip at the resolution's overview hop.
        public var detail: SpectrogramDetail?

        public init(clip: String, resolution: SpectrogramResolution, detail: SpectrogramDetail? = nil) {
            self.clip = clip
            self.resolution = resolution
            self.detail = detail
        }
    }

    /// A small least-recently-used store.
    struct Store {
        let capacity: Int
        var images: [Key: SpectrogramImage] = [:]
        var order: [Key] = []

        mutating func touch(_ key: Key) {
            order.removeAll { $0 == key }
            order.append(key)
        }

        mutating func insert(_ key: Key, _ image: SpectrogramImage) {
            images[key] = image
            touch(key)
            while order.count > capacity { images[order.removeFirst()] = nil }
        }
    }

    private var overviews = Store(capacity: overviewCapacity)
    private var details = Store(capacity: detailCapacity)

    public init() {}

    public func image(_ key: Key) -> SpectrogramImage? {
        key.detail == nil ? overviews.images[key] : details.images[key]
    }

    /// The most recent detail of this clip and resolution that covers `range` (shown while the
    /// detail for a new zoom level computes, instead of falling back to the blurry overview).
    public func latestDetail(clip: String, resolution: SpectrogramResolution, covering range: ClosedRange<Double>)
        -> SpectrogramImage?
    {
        for key in details.order.reversed() where key.clip == clip && key.resolution == resolution {
            if let img = details.images[key], img.extent.lowerBound <= range.lowerBound,
                img.extent.upperBound >= range.upperBound
            {
                return img
            }
        }
        return nil
    }

    /// Drop everything (the review window closed; the app lives on in the menu bar).
    public func removeAll() {
        overviews = Store(capacity: Self.overviewCapacity)
        details = Store(capacity: Self.detailCapacity)
    }

    /// Compute `key` for `audio` unless cached. Cancelling the calling task cancels the work.
    /// A detail render passes the overview's `topDb` so both share one colour mapping.
    public func load(_ key: Key, audio: AudioClip, top: Float? = nil) async {
        if image(key) != nil {
            if key.detail == nil { overviews.touch(key) } else { details.touch(key) }
            return
        }
        let job = Task.detached(priority: .userInitiated) { Self.render(key, audio: audio, top: top) }
        let result = await withTaskCancellationHandler { await job.value } onCancel: { job.cancel() }
        guard !Task.isCancelled, let result else { return }
        if key.detail == nil { overviews.insert(key, result) } else { details.insert(key, result) }
    }

    /// The STFT + colour mapping (any thread). `nil` when the task is cancelled.
    nonisolated static func render(_ key: Key, audio: AudioClip, top: Float? = nil) -> SpectrogramImage? {
        let sr = audio.sampleRate
        let fft = key.resolution.fftSize(sampleRate: sr)
        let spec: Spectrogram?
        if let d = key.detail {
            spec = Spectrogram.compute(
                samples: audio.mono, sampleRate: sr, fftSize: fft, hop: d.hop, start: d.start, frames: d.frames,
                isCancelled: { Task.isCancelled })
        } else {
            spec = Spectrogram.compute(
                samples: audio.mono, sampleRate: sr, fftSize: fft, hop: key.resolution.hop(sampleRate: sr),
                isCancelled: { Task.isCancelled })
        }
        guard let spec else { return nil }
        let scale = FrequencyScale.speech(sampleRate: sr)
        // Top of the colour map: the clip's loudest bin (a detail's own max could be quieter).
        let top = top ?? spec.maxDb
        guard let img = spec.image(scale: scale, rows: rows, top: top, isCancelled: { Task.isCancelled }) else { return nil }
        return SpectrogramImage(
            image: img, extent: spec.extent, frames: spec.frames, hop: spec.hop, sampleRate: sr, scale: scale, topDb: top)
    }
}
