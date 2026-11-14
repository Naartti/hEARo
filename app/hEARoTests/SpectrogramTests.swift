import CoreGraphics
import Foundation
import Testing

@testable import HearoKit

@Suite struct SpectrogramTests {
    let sr = 48_000.0

    func sine(_ hz: Double, seconds: Double = 0.5, amplitude: Float = 0.5) -> [Float] {
        (0..<Int(seconds * sr)).map { amplitude * Float(sin(2 * .pi * hz * Double($0) / sr)) }
    }

    @Test func dimensionsFollowHopAndFFTSize() throws {
        let x = [Float](repeating: 0, count: 48_000)
        let s = try #require(Spectrogram.compute(samples: x, sampleRate: sr, fftSize: 1024, hop: 128))
        #expect(s.bins == 513)
        #expect(s.frames == (48_000 - 1) / 128 + 1)
        #expect(s.db.count == s.frames * s.bins)
        #expect(abs(s.time(ofFrame: 10) - 1280 / sr) < 1e-12)
        #expect(abs(s.extent.lowerBound + 64 / sr) < 1e-12)
        let img = try #require(s.image(scale: .speech(sampleRate: sr), rows: 200))
        #expect(img.width == s.frames)
        #expect(img.height == 200)
    }

    @Test func sineAt1kHzPeaksInThe1kHzBinAndRow() throws {
        let s = try #require(Spectrogram.compute(samples: sine(1000), sampleRate: sr, fftSize: 1024, hop: 128))
        let f = s.frames / 2
        let peak = (0..<s.bins).max { s.value(frame: f, bin: $0) < s.value(frame: f, bin: $1) }!
        #expect(abs(s.frequency(ofBin: peak) - 1000) <= s.binHz)
        // Calibrated: a −6 dBFS (0.5) sine reads about −6 dB (within the Hann scalloping loss).
        #expect(s.value(frame: f, bin: peak) > -6.03 - 1.5)
        #expect(s.value(frame: f, bin: peak) < -6.03 + 0.5)

        let scale = FrequencyScale.speech(sampleRate: sr)
        let rows = 240
        let grid = s.grid(scale: scale, rows: rows)
        let column = (0..<rows).map { grid[$0 * s.frames + f] }
        let best = column.indices.max { column[$0] < column[$1] }!
        #expect(abs(Double(best) + 0.5 - scale.row(ofHz: 1000, rows: rows)) <= 1.5)
        // Far from the tone is ~80 dB down (Hann sidelobes).
        let far = Int(scale.row(ofHz: 100, rows: rows))
        #expect(column[far] < column[best] - 60)
    }

    @Test func clickIsBroadbandInOneFrameOnly() throws {
        var x = [Float](repeating: 0, count: 24_000)
        let at = 12_000
        x[at] = 1
        let hop = 128
        let s = try #require(Spectrogram.compute(samples: x, sampleRate: sr, fftSize: 1024, hop: hop))
        let centre = Int((Double(at) / Double(hop)).rounded())  // frame whose centre is nearest
        for hz in [100.0, 1000, 5000, 10_000, 18_000] {
            let k = Int((hz / s.binHz).rounded())
            #expect(s.value(frame: centre, bin: k) > s.maxDb - 6, "click energy at \(hz) Hz")
        }
        // A frame whose window doesn't reach the click is silent.
        let away = centre + 1024 / hop + 1
        #expect(s.value(frame: away, bin: 100) < -190)
        // The loudest frame (summed over bins) is the click's.
        let energies = (0..<s.frames).map { fr in (0..<s.bins).reduce(Float(0)) { $0 + max(0, s.value(frame: fr, bin: $1) + 120) } }
        #expect(energies.indices.max { energies[$0] < energies[$1] } == centre)
    }

    @Test func silenceIsTheFloor() throws {
        let s = try #require(Spectrogram.compute(samples: [Float](repeating: 0, count: 9_600), sampleRate: sr))
        #expect(s.db.allSatisfy { $0 < -190 })
        #expect(s.maxDb < -190)
        // Renders as the darkest colour everywhere.
        let img = try #require(s.image(scale: .speech(sampleRate: sr), rows: 32))
        let data = try #require(img.dataProvider?.data as Data?)
        let lut = Colormap.magma
        for p in stride(from: 0, to: data.count, by: 4) {
            #expect(data[p] == lut[0] && data[p + 1] == lut[1] && data[p + 2] == lut[2])
            if data[p] != lut[0] { break }
        }
    }

    @Test func offsetFramesCentreOnTheirSamples() throws {
        // A detail render (start/hop) puts a click in the frame whose centre is nearest.
        var x = [Float](repeating: 0, count: 48_000)
        x[30_000] = 1
        let s = try #require(
            Spectrogram.compute(samples: x, sampleRate: sr, fftSize: 512, hop: 8, start: 29_000, frames: 250))
        let energies = (0..<s.frames).map { fr in (0..<s.bins).reduce(Float(0)) { $0 + max(0, s.value(frame: fr, bin: $1) + 120) } }
        let best = energies.indices.max { energies[$0] < energies[$1] }!
        #expect(abs(s.time(ofFrame: best) - 30_000 / sr) <= 8 / sr)
    }

    @Test func logScaleRoundTripsAndTicks() {
        let scale = FrequencyScale.speech(sampleRate: sr)
        #expect(scale.maxHz == 20_000)
        #expect(FrequencyScale.speech(sampleRate: 16_000).maxHz == 8000)
        for hz in [30.0, 50, 60, 100, 440, 1000, 5000, 10_000, 20_000] {
            #expect(abs(scale.hz(atFraction: scale.fraction(ofHz: hz)) - hz) < 1e-6 * hz)
            #expect(abs(scale.hz(atRow: scale.row(ofHz: hz, rows: 300), rows: 300) - hz) < 1e-6 * hz)
        }
        #expect(scale.row(ofHz: 20_000, rows: 300) == 0)
        #expect(abs(scale.row(ofHz: 30, rows: 300) - 300) < 1e-9)
        #expect(scale.row(ofHz: 100, rows: 300) > scale.row(ofHz: 1000, rows: 300))
        // Log: each decade gets the same height.
        let d1 = scale.row(ofHz: 100, rows: 300) - scale.row(ofHz: 1000, rows: 300)
        let d2 = scale.row(ofHz: 1000, rows: 300) - scale.row(ofHz: 10_000, rows: 300)
        #expect(abs(d1 - d2) < 1e-9)
        #expect([100.0, 1000, 5000, 10_000].allSatisfy(scale.ticks.contains))
        #expect(FrequencyScale.label(5000) == "5k")
        #expect(FrequencyScale.label(100) == "100")
    }

    @Test func resolutionsAt48kHz() {
        #expect(SpectrogramResolution.time.fftSize(sampleRate: 48_000) == 512)
        #expect(SpectrogramResolution.balanced.fftSize(sampleRate: 48_000) == 1024)
        #expect(SpectrogramResolution.frequency.fftSize(sampleRate: 48_000) == 4096)
        #expect(SpectrogramResolution.balanced.hop(sampleRate: 48_000) == 128)
        #expect(SpectrogramResolution.balanced.fftSize(sampleRate: 44_100) == 1024)
    }

    @Test func colormapIsMonotonicInLightness() {
        let lut = Colormap.magma
        #expect(lut.count == 768)
        // Rec. 709 luma rises from black to pale yellow.
        let luma = (0..<256).map { 0.2126 * Double(lut[$0 * 3]) + 0.7152 * Double(lut[$0 * 3 + 1]) + 0.0722 * Double(lut[$0 * 3 + 2]) }
        #expect(zip(luma, luma.dropFirst()).allSatisfy { $0 <= $1 + 0.5 })
        #expect(luma[0] < 10 && luma[255] > 230)
    }

    @Test func cancellationStopsEarly() {
        let x = [Float](repeating: 0.1, count: 48_000)
        #expect(Spectrogram.compute(samples: x, sampleRate: sr, isCancelled: { true }) == nil)
    }

    @Test func eightSecondClipIsFast() throws {
        let clip = AudioClip.synthetic()
        let res = SpectrogramResolution.balanced
        let t0 = ContinuousClock.now
        let s = try #require(
            Spectrogram.compute(
                samples: clip.mono, sampleRate: clip.sampleRate, fftSize: res.fftSize(sampleRate: clip.sampleRate),
                hop: res.hop(sampleRate: clip.sampleRate)))
        _ = try #require(s.image(scale: .speech(sampleRate: clip.sampleRate), rows: SpectrogramCache.rows))
        let elapsed = ContinuousClock.now - t0
        // Measured on Apple silicon (2026-11-14): ~14 ms release, ~320 ms debug (unoptimised
        // Swift loops). Bounds leave headroom for a loaded machine.
        #if DEBUG
            let budget = Duration.milliseconds(1500)
        #else
            let budget = Duration.milliseconds(200)
        #endif
        #expect(elapsed < budget, "8 s clip took \(elapsed)")
    }

    // MARK: Deep zoom

    @Test func noDetailWhileTheOverviewIsSharpEnough() {
        // Whole 8 s clip on 1600 px: 240 samples per pixel, overview hop 128.
        let vp = Viewport(total: 8)
        #expect(SpectrogramDetail.plan(viewport: vp, pixels: 1600, sampleRate: sr, overviewHop: 128, fftSize: 1024, clipSamples: 384_000) == nil)
    }

    @Test func detailCoversTheViewAndIgnoresSmallPans() throws {
        var vp = Viewport(total: 8)
        vp.show(4.0...4.05, padding: 0)  // 50 ms on 1600 px: 1.5 samples per pixel
        let d = try #require(
            SpectrogramDetail.plan(viewport: vp, pixels: 1600, sampleRate: sr, overviewHop: 128, fftSize: 1024, clipSamples: 384_000))
        #expect(d.hop == 1)
        #expect(Double(d.start) <= vp.start * sr)
        #expect(Double(d.start + d.frames * d.hop) >= vp.end * sr)
        #expect(d.frames < 4 * 1600)  // bounded work
        vp.pan(by: 0.001)
        #expect(SpectrogramDetail.plan(viewport: vp, pixels: 1600, sampleRate: sr, overviewHop: 128, fftSize: 1024, clipSamples: 384_000) == d)
        // Zooming out past a power of two coarsens the hop.
        vp.zoom(by: 0.25, around: 4.0)
        let coarser = try #require(
            SpectrogramDetail.plan(viewport: vp, pixels: 1600, sampleRate: sr, overviewHop: 128, fftSize: 1024, clipSamples: 384_000))
        #expect(coarser.hop == 4)
    }

    @Test func detailStaysInsideTheClip() throws {
        var vp = Viewport(total: 1)
        vp.show(0.99...1.0, padding: 0)
        let d = try #require(
            SpectrogramDetail.plan(viewport: vp, pixels: 800, sampleRate: sr, overviewHop: 128, fftSize: 1024, clipSamples: 48_000))
        #expect(d.start >= 0)
        #expect(d.start + (d.frames - 1) * d.hop < 48_000 + d.hop * SpectrogramDetail.block)
    }

    @Test func sliceMapsTheViewToImageColumns() throws {
        let s = try #require(Spectrogram.compute(samples: sine(440, seconds: 1), sampleRate: sr, fftSize: 1024, hop: 480))
        let img = try #require(s.image(scale: .speech(sampleRate: sr), rows: 64))
        let si = SpectrogramImage(
            image: img, extent: s.extent, frames: s.frames, hop: s.hop, sampleRate: sr, scale: .speech(sampleRate: sr),
            topDb: s.maxDb)
        let slice = try #require(si.slice(from: 0.5, to: 0.6))  // 10 ms columns
        #expect(slice.columns.lowerBound <= 50 && slice.columns.upperBound >= 60)
        #expect(slice.span.lowerBound <= 0.5 && slice.span.upperBound >= 0.6)
        #expect(si.slice(from: 2, to: 3)?.columns == nil)
    }

    @Test func frequencyDetailSkipsDetailWhereTheWindowDominates() {
        // 4096-pt window: the whole clip at 240 samples/pixel gains nothing from a finer hop.
        let vp = Viewport(total: 8)
        #expect(
            SpectrogramDetail.plan(viewport: vp, pixels: 1600, sampleRate: sr, overviewHop: 512, fftSize: 4096, clipSamples: 384_000)
                == nil)
    }

    // MARK: Row mapping and levels

    @Test func rowMapSwitchesFromBandsToInterpolationOnce() {
        let scale = FrequencyScale.speech(sampleRate: sr)
        let binHz = sr / 1024
        let map = Spectrogram.RowMap(scale: scale, rows: 320, binHz: binHz, bins: 513)
        #expect(map.bandRows > 0 && map.bandRows < 320)
        #expect(map.bandRows + map.position.count == 320)
        // Bands at the top span ≥ 2 bins and descend; interpolated positions keep descending.
        #expect(map.len.allSatisfy { $0 >= 2 })
        #expect(zip(map.lo, map.lo.dropFirst()).allSatisfy { $0 >= $1 })
        #expect(zip(map.position, map.position.dropFirst()).allSatisfy { $0 > $1 })
        #expect(Float(map.lo.last!) >= map.position.first!)
        #expect(map.position.allSatisfy { $0 >= 0 && $0 < 512 })
    }

    @Test func dcReadsLikeAFullScaleSine() throws {
        // A constant 0.5 reads ≈ −6 dB, like a 0.5-amplitude sine (DC isn't double-counted).
        let s = try #require(
            Spectrogram.compute(samples: [Float](repeating: 0.5, count: 8192), sampleRate: sr, fftSize: 1024, hop: 512))
        let mid = s.frames / 2
        #expect(abs(s.value(frame: mid, bin: 0) - 20 * log10(Float(0.5))) < 1)
    }

    // MARK: Cache

    @MainActor @Test func deepZoomNeverEvictsTheOverview() async throws {
        let cache = SpectrogramCache()
        let clip = AudioClip(mono: sine(1000, seconds: 0.5), sampleRate: sr)
        let overview = SpectrogramCache.Key(clip: "a", resolution: .balanced)
        await cache.load(overview, audio: clip)
        let top = try #require(cache.image(overview)).topDb
        for i in 0..<12 {
            let d = SpectrogramDetail(start: i * 256, frames: 256, hop: 1)
            await cache.load(.init(clip: "a", resolution: .balanced, detail: d), audio: clip, top: top)
        }
        #expect(cache.image(overview) != nil)
        // Only the last few details are kept; the newest covering a range is found.
        let first = SpectrogramCache.Key(clip: "a", resolution: .balanced, detail: SpectrogramDetail(start: 0, frames: 256, hop: 1))
        #expect(cache.image(first) == nil)
        let newest = try #require(cache.latestDetail(clip: "a", resolution: .balanced, covering: (11 * 256 + 10) / sr...(11 * 256 + 20) / sr))
        #expect(newest.extent.lowerBound < (11 * 256 + 10) / sr)
        #expect(cache.latestDetail(clip: "b", resolution: .balanced, covering: 0...0.001) == nil)
        cache.removeAll()
        #expect(cache.image(overview) == nil)
    }

    @MainActor @Test func overviewsAreEvictedLeastRecentlyUsedFirst() async {
        let cache = SpectrogramCache()
        let clip = AudioClip(mono: sine(500, seconds: 0.1), sampleRate: sr)
        let keys = (0...SpectrogramCache.overviewCapacity).map { SpectrogramCache.Key(clip: "c\($0)", resolution: .time) }
        for k in keys.dropLast() { await cache.load(k, audio: clip) }
        await cache.load(keys[0], audio: clip)  // a hit refreshes recency
        await cache.load(keys.last!, audio: clip)
        #expect(cache.image(keys[0]) != nil)
        #expect(cache.image(keys[1]) == nil)
    }

    @MainActor @Test func cancelledLoadStoresNothing() async {
        let cache = SpectrogramCache()
        let clip = AudioClip(mono: [Float](repeating: 0.1, count: 48_000 * 8), sampleRate: sr)
        let key = SpectrogramCache.Key(clip: "x", resolution: .time)
        let task = Task { await cache.load(key, audio: clip) }
        task.cancel()
        await task.value
        #expect(cache.image(key) == nil)
    }
}
