import AVFoundation
import Foundation
import Testing

@testable import HearoKit

/// Records what the inspector asks of the player.
@MainActor
final class FakePlayer: ClipPlayback {
    var isPlaying = false
    var looping = false
    var currentTime: Double?
    private(set) var playsProcessed = false
    private(set) var hasProcessed = false
    private(set) var processedLoads: [DecodedAudio?] = []
    func load(_ url: URL) throws { hasProcessed = false }
    func loadProcessed(_ audio: DecodedAudio?) throws {
        processedLoads.append(audio)
        hasProcessed = audio != nil
    }
    func setPlaysProcessed(_ on: Bool) { playsProcessed = on }
    func play(from start: Double, to end: Double?, loop: Bool) throws { isPlaying = true }
    func stop() { isPlaying = false }
}

/// A small float WAV (a ramp) for player and renderer tests.
func writeWav(_ url: URL, frames: Int, channels: Int = 1, sampleRate: Double = 48_000) throws {
    let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: AVAudioChannelCount(channels))!
    let file = try AVAudioFile(forWriting: url, settings: format.settings)
    let b = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames))!
    for c in 0..<channels {
        for i in 0..<frames { b.floatChannelData![c][i] = Float(i) / Float(frames) * 0.5 }
    }
    b.frameLength = AVAudioFrameCount(frames)
    try file.write(from: b)
}

/// A renderer that records each call's settings and can be slow on chosen calls.
final class RecordingRenderer: ClipRenderer, @unchecked Sendable {
    private let lock = NSLock()
    private var _calls: [EngineSettings] = []
    /// Seconds to block on call n (0-based); missing = no delay.
    let delays: [Int: Double]
    var fail = false

    init(delays: [Int: Double] = [:]) { self.delays = delays }

    var calls: [EngineSettings] { lock.withLock { _calls } }

    func render(_ input: URL, to output: URL, settings: EngineSettings, bypass: Bool, columnsPerSecond: Float) throws
        -> RenderResult
    {
        let n = lock.withLock {
            _calls.append(settings)
            return _calls.count - 1
        }
        if let d = delays[n] { Thread.sleep(forTimeInterval: d) }
        if fail { throw CocoaError(.fileReadCorruptFile) }
        try writeWav(output, frames: 480)
        return RenderResult(
            output: output, sampleRate: 48_000, channels: 1, frames: 48_000, columnsPerSecond: columnsPerSecond,
            inputPeaks: [0.5], outputPeaks: [0.25], reductions: [ModuleReduction(key: "desmack", gainReductionDb: [6])],
            events: [], renderMs: Double(n))
    }
}

@MainActor
struct ReviewInspectorTests {
    let clip = URL(fileURLWithPath: "/tmp/hearo-test-clip.wav")

    private func make(_ renderer: RecordingRenderer, debounce: Duration = .milliseconds(20))
        -> (ReviewInspectorModel, AppModel, FakeEngine, FakePlayer)
    {
        let engine = FakeEngine()
        let app = AppModel(engine: engine)
        let player = FakePlayer()
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("hearo-review-test-\(UUID().uuidString)")
        let m = ReviewInspectorModel(renderer: renderer, live: app, player: player, debounce: debounce, folder: folder)
        return (m, app, engine, player)
    }

    @Test func startsFromLiveAndRendersTheOpenedClip() async {
        let r = RecordingRenderer()
        let (fresh, app, _, player) = make(r)
        #expect(fresh.settings == app.liveSettings && fresh.matchesLive)
        app.setParam("desmack", "range", 6)  // changed live while the window was closed
        fresh.windowAppeared()
        #expect(fresh.matchesLive && !fresh.edited)
        fresh.open(clip)
        #expect(fresh.state == .rendering)
        await fresh.waitUntilIdle()
        #expect(r.calls.count == 1 && r.calls[0].module("desmack")?.value("range") == 6)
        #expect(fresh.state == .ready(ms: 0))
        #expect(player.hasProcessed && player.playsProcessed, "the render plays unless bypassed")
        #expect(fresh.overlay != nil)
        fresh.windowClosed()
    }

    @Test func editsRenderOnceAfterTheDebounce() async {
        let r = RecordingRenderer()
        let (m, _, engine, _) = make(r)
        m.open(clip)
        await m.waitUntilIdle()
        for v in stride(from: Float(2), through: 20, by: 2) { m.set("desmack", "range", v) }
        m.setEnabled("deess", false)
        await m.waitUntilIdle()
        #expect(r.calls.count == 2, "one render for the burst of edits")
        #expect(r.calls.last?.module("desmack")?.value("range") == 20)
        #expect(r.calls.last?.module("deess")?.enabled == false)
        #expect(m.edited && !m.matchesLive)
        #expect(!engine.calls.contains("applySettings"), "live sound untouched")
        m.windowClosed()
    }

    @Test func staleRenderResultIsDropped() async throws {
        // First render (after opening) is slow; an edit meanwhile starts a fast one.
        let r = RecordingRenderer(delays: [0: 0.3])
        let (m, _, _, player) = make(r, debounce: .zero)
        m.open(clip)
        let stale = m.renderTask
        try await Task.sleep(for: .milliseconds(50))
        m.set("desmack", "range", 6)
        await m.waitUntilIdle()
        await stale?.value
        #expect(r.calls.count == 2)
        #expect(m.state == .ready(ms: 1), "the newer render's result stays")
        #expect(player.processedLoads.count == 1, "the stale result was never loaded")
        m.windowClosed()
    }

    /// Holding ↓ through the clip list must not start a render per clip.
    @Test func browsingClipsQuicklyRendersAtMostTwice() async {
        let r = RecordingRenderer(delays: [0: 0.1])
        let (m, _, _, player) = make(r, debounce: .milliseconds(30))
        for i in 0..<10 { m.open(URL(fileURLWithPath: "/tmp/hearo-test-clip-\(i).wav")) }
        await m.waitUntilIdle()
        #expect(r.calls.count == 1, "only the last clip after the debounce")
        #expect(player.hasProcessed)
        m.windowClosed()
    }

    /// While a render runs (it can't be interrupted), later requests queue: one more render,
    /// with the newest settings, instead of one per request.
    @Test func onlyOneRenderRunsAtATime() async throws {
        let r = RecordingRenderer(delays: [0: 0.3])
        let (m, _, _, _) = make(r, debounce: .milliseconds(5))
        m.open(clip)
        try await Task.sleep(for: .milliseconds(30))  // render 0 is running
        for v in stride(from: Float(1), through: 5, by: 1) {
            m.set("deess", "range", v)
            try await Task.sleep(for: .milliseconds(20))  // each past the debounce
        }
        await m.waitUntilIdle()
        #expect(r.calls.count == 2)
        #expect(r.calls.last?.module("deess")?.value("range") == 5, "the newest request wins")
        #expect(m.state == .ready(ms: 1))
        m.windowClosed()
    }

    @Test func bypassSwitchesPlaybackAndHidesTheOverlay() async {
        let r = RecordingRenderer()
        let (m, _, _, player) = make(r)
        m.open(clip)
        await m.waitUntilIdle()
        #expect(player.playsProcessed && m.overlay != nil)
        m.toggleBypass()
        #expect(m.bypass && !player.playsProcessed && m.overlay == nil)
        #expect(r.calls.count == 1, "bypass doesn't re-render")
        m.toggleBypass()
        #expect(player.playsProcessed)
        // A new render while bypassed keeps playing the original.
        m.setBypass(true)
        m.set("deess", "range", 3)
        await m.waitUntilIdle()
        #expect(!player.playsProcessed)
        m.windowClosed()
    }

    @Test func applyToLiveAndUseLive() async {
        let r = RecordingRenderer()
        let (m, app, engine, _) = make(r)
        m.set("desmack", "range", 30)
        m.setEnabled("dehum", false)
        m.applyToLive()
        #expect(engine.calls.last == "applySettings")
        #expect(app.liveSettings.module("desmack")?.value("range") == 30)
        #expect(app.snapshot.modules.first { $0.id == "dehum" }?.enabled == false)
        #expect(m.matchesLive && !m.edited)
        m.set("desmack", "range", 3)
        m.useLiveSettings()
        #expect(m.settings.module("desmack")?.value("range") == 30)
    }

    @Test func failedRenderIsShownAndPlaysTheOriginal() async {
        let r = RecordingRenderer()
        r.fail = true
        let (m, _, _, player) = make(r)
        m.open(clip)
        await m.waitUntilIdle()
        guard case .failed = m.state else {
            Issue.record("expected a failure, got \(m.state)")
            return
        }
        #expect(m.result == nil && !player.hasProcessed)
        m.open(nil)
        #expect(m.state == .idle)
    }

    @Test func reviewModelWiresTheInspector() {
        let lib = SnapshotLibrary(folder: URL(fileURLWithPath: "/nonexistent"))
        #expect(ReviewModel(library: lib).inspector == nil, "no engine, no inspector")
        let app = AppModel(engine: FakeEngine())
        #expect(ReviewModel(library: lib, live: app, renderer: FakeClipRenderer()).inspector != nil)
    }
}

/// A/B switching and looping keep the playhead: the scheduling maths behind `ClipPlayer`.
struct PlaybackPlanTests {
    @Test func switchingMidSpanContinuesFromTheSamePosition() {
        let plan = ClipPlayer.plan(span: 1000..<5000, at: 3000, loop: false)
        #expect(plan == .init(first: 3000..<5000, loop: nil, offset: 2000))
        #expect(ClipPlayer.position(span: 1000..<5000, offset: plan.offset, played: 0, loop: false) == 3000)
        #expect(ClipPlayer.position(span: 1000..<5000, offset: plan.offset, played: 500, loop: false) == 3500)
        #expect(ClipPlayer.position(span: 1000..<5000, offset: plan.offset, played: 99_999, loop: false) == 5000, "stops at the end")
    }

    @Test func loopsResumeMidLoopThenRepeatTheWholeRegion() {
        let plan = ClipPlayer.plan(span: 1000..<2000, at: 1600, loop: true)
        #expect(plan == .init(first: 1600..<2000, loop: 1000..<2000, offset: 600))
        #expect(ClipPlayer.position(span: 1000..<2000, offset: 600, played: 300, loop: true) == 1900)
        #expect(ClipPlayer.position(span: 1000..<2000, offset: 600, played: 400, loop: true) == 1000, "wraps to the start")
        #expect(ClipPlayer.position(span: 1000..<2000, offset: 600, played: 1500, loop: true) == 1100)
        let fromStart = ClipPlayer.plan(span: 1000..<2000, at: 1000, loop: true)
        #expect(fromStart == .init(first: nil, loop: 1000..<2000, offset: 0), "no partial piece at the start")
    }

    @Test func positionsOutsideTheSpanAreClamped() {
        #expect(ClipPlayer.plan(span: 100..<200, at: 50, loop: false).offset == 0)
        #expect(ClipPlayer.plan(span: 100..<200, at: 500, loop: true).offset == 99)
        #expect(ClipPlayer.plan(span: 0..<0, at: 0, loop: true) == .init(first: nil, loop: nil, offset: 0))
    }
}

/// Bands for a stretch of a render (review waveform).
@MainActor
struct RenderBandsTests {
    @Test func aggregatesTheDeepestCutAndLoudestPeakOverTheStretch() {
        let r = RenderResult(
            output: URL(fileURLWithPath: "/dev/null"), sampleRate: 48_000, channels: 1, frames: 400, columnsPerSecond: 100,
            inputPeaks: [0.1, 0.5, 0.2, 0.1], outputPeaks: [0.1, 0.125, 0.2, 0.1],
            reductions: [
                ModuleReduction(key: "deess", gainReductionDb: [0, 0, 3, 0]),
                ModuleReduction(key: "desmack", gainReductionDb: [0, 12, 0, 0]),
            ], events: [])
        let b = r.bands(from: 0.0, to: 0.03)
        #expect(b.bands.map(\.module) == [3, 4], "by key, chain order: de-smack then de-ess")
        #expect(abs(b.core - 0.25) < 0.01, "output 0.125 of input 0.5")
        #expect(r.bands(from: 0.035, to: 0.04) == .none)
        #expect(r.bands(from: 5, to: 6) == .none, "past the end")
    }

    @Test func laneSummaryNamesWhatWasRemoved() {
        let r = RenderResult(
            output: URL(fileURLWithPath: "/dev/null"), sampleRate: 48_000, channels: 1, frames: 1, columnsPerSecond: 100,
            inputPeaks: [], outputPeaks: [], reductions: [],
            events: [
                RenderEvent(module: "deess", kind: "sibilance", start: 0, duration: 0.1, reductionDb: 6),
                RenderEvent(module: "desmack", kind: "smack", start: 1, duration: 0.01, reductionDb: 12),
                RenderEvent(module: "deess", kind: "sibilance", start: 2, duration: 0.1, reductionDb: 6),
            ])
        let names = ["desmack": "De-smack", "deess": "De-ess"]
        #expect(
            WaveformLane.summary(regions: 2, proposals: 1, overlay: r, names: names)
                == "2 marked regions, 1 proposals; removed: De-smack 1, De-ess 2")
        #expect(WaveformLane.summary(regions: 0, proposals: 0, overlay: nil, names: names) == "0 marked regions, 0 proposals")
    }
}

/// The live waveform's VoiceOver value and key-mapped reduction rows.
@MainActor
struct LiveWaveformTests {
    @Test func summaryNamesModulesAndDepths() {
        var b = WaveformBuffer(capacity: 300)
        b.append(FakeEngine.syntheticWaveform(columns: 300, start: 0))
        let s = WaveformView.summary(b, names: ["desmack": "De-smack", "deess": "De-ess"])
        #expect(s.hasPrefix("Recently turned down: "))
        #expect(s.contains("De-smack up to 18 dB") && s.contains("De-ess up to"))
        #expect(!s.contains("dehum"), "0.75 dB hum is below the 1 dB mention threshold")
        #expect(WaveformView.summary(WaveformBuffer(capacity: 10), names: [:]) == "Nothing removed recently")
    }

    @Test func reductionRowsAreMatchedByKeyNotPosition() {
        var b = WaveformBuffer(capacity: 4)
        b.append(
            WaveformChunk(
                next: 1, peaks: [0.5], removed: [true], outputPeaks: [0.2],
                reductions: [
                    ModuleReduction(key: "deess", gainReductionDb: [3]),
                    ModuleReduction(key: "desmack", gainReductionDb: [9]),
                ]))
        #expect(b.columns[0].reductionDb == [0, 0, 0, 9, 3])
    }
}

/// `ClipPlayer` A/B buffers (no audio is played: the engine only starts on `play`).
@MainActor
struct ClipPlayerBufferTests {
    private func temp(_ name: String) -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent("hearo-player-\(UUID().uuidString)-\(name).wav")
    }

    @Test func processedMustLineUpWithTheOriginal() throws {
        let original = temp("orig")
        let short = temp("short")
        defer { for u in [original, short] { try? FileManager.default.removeItem(at: u) } }
        try writeWav(original, frames: 4800, channels: 2)
        try writeWav(short, frames: 4000, channels: 2)
        let p = ClipPlayer()
        try p.load(original)
        #expect(throws: ClipPlayer.ClipPlayerError.self) { try p.loadProcessed(try DecodedAudio(contentsOf: short)) }
        #expect(throws: ClipPlayer.ClipPlayerError.self) {
            try p.loadProcessed(DecodedAudio(sampleRate: 48_000, channels: [[Float](repeating: 0, count: 4800)]))
        }
        #expect(!p.hasProcessed)
        let same = try DecodedAudio(contentsOf: original)
        #expect(same.frames == 4800 && same.channels.count == 2 && same.sampleRate == 48_000)
        try p.loadProcessed(same)
        #expect(p.hasProcessed)
        p.setPlaysProcessed(true)
        #expect(p.playsProcessed && !p.isPlaying, "switching while stopped doesn't start playback")
        try p.loadProcessed(nil)
        #expect(!p.hasProcessed)
        try p.loadProcessed(same)
        try p.load(short)
        #expect(!p.hasProcessed, "a new clip drops the old render")
    }
}

struct LossySettingsTests {
    /// One malformed value drops that value, not the whole file.
    @Test func badElementsAreSkipped() throws {
        let json =
            #"{"modules":[{"key":"desmack","params":[{"key":"range","value":"12"},{"key":"threshold","value":20}]},{"nokey":1},{"key":"deess","enabled":false}]}"#
        let s = try JSONDecoder().decode(EngineSettings.self, from: Data(json.utf8))
        #expect(s.modules.map(\.key) == ["desmack", "deess"])
        #expect(s.module("desmack")?.params == [ParamValue(key: "threshold", value: 20)])
        #expect(s.module("deess")?.enabled == false)
    }
}
