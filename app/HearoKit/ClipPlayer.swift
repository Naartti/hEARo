import AVFoundation
import Foundation
import Observation

/// What the review window needs from a player (`ClipPlayer`; a fake in tests).
@MainActor
public protocol ClipPlayback: AnyObject {
    var isPlaying: Bool { get }
    var looping: Bool { get }
    /// Where playback is now (seconds in the clip), nil when stopped.
    var currentTime: Double? { get }
    /// Whether the processed version plays (when one is loaded); otherwise the original.
    var playsProcessed: Bool { get }
    var hasProcessed: Bool { get }
    /// Open a clip's original WAV (drops any processed version).
    func load(_ url: URL) throws
    /// The processed version of the open clip (same length, rate and channels; nil drops it).
    /// Decode it off the main thread (`DecodedAudio(contentsOf:)`). While it is playing, the new
    /// audio continues from the same position.
    func loadProcessed(_ audio: DecodedAudio?) throws
    /// Switch between original and processed; while playing, at the same position and span.
    func setPlaysProcessed(_ on: Bool)
    /// Play `from…to` (seconds; `to` nil = clip end), looping if asked.
    func play(from start: Double, to end: Double?, loop: Bool) throws
    func stop()
}

extension ClipPlayback {
    public func play(from start: Double) throws { try play(from: start, to: nil, loop: false) }
}

/// A WAV decoded to planar float samples: Sendable, so it can be read on a background thread
/// and handed to the player (which only copies it into a buffer on the main thread).
public struct DecodedAudio: Sendable, Equatable {
    public var sampleRate: Double
    /// One array per channel, all the same length.
    public var channels: [[Float]]
    public var frames: Int { channels.first?.count ?? 0 }

    public init(sampleRate: Double, channels: [[Float]]) {
        self.sampleRate = sampleRate
        self.channels = channels
    }

    public init(contentsOf url: URL) throws {
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let frames = AVAudioFrameCount(file.length)
        guard let b = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: max(frames, 1)) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        if frames > 0 { try file.read(into: b, frameCount: frames) }
        let n = Int(b.frameLength)
        let chans = Int(b.format.channelCount)
        guard let data = b.floatChannelData else { throw CocoaError(.fileReadCorruptFile) }
        self.init(
            sampleRate: b.format.sampleRate,
            channels: (0..<chans).map { Array(UnsafeBufferPointer(start: data[$0], count: n)) })
    }
}

/// Plays a snapshot WAV for the review window — the original as saved, or the processed version
/// rendered by the review inspector — optionally looping a span sample-accurately. Switching
/// between the two (A/B) keeps the position: both buffers line up sample for sample.
/// AVAudioEngine runs only while playing so hEARo doesn't hold the output device open. hEARo's
/// own process is excluded from its tap, so what you hear is never processed twice.
@MainActor
@Observable
public final class ClipPlayer: ClipPlayback {
    public private(set) var isPlaying = false
    public private(set) var loadedURL: URL?
    /// The span being played (seconds) and whether it loops.
    public private(set) var span: ClosedRange<Double>?
    public private(set) var looping = false
    public private(set) var playsProcessed = false
    public private(set) var hasProcessed = false

    @ObservationIgnored private let engine = AVAudioEngine()
    @ObservationIgnored private let node = AVAudioPlayerNode()
    @ObservationIgnored private var original: AVAudioPCMBuffer?
    @ObservationIgnored private var processed: AVAudioPCMBuffer?
    /// The span in frames and where in it the current schedule started.
    @ObservationIgnored private var spanFrames: Range<Int> = 0..<0
    @ObservationIgnored private var startOffset = 0
    @ObservationIgnored private var generation = 0

    public init() {
        engine.attach(node)
    }

    public var duration: Double {
        guard let b = original else { return 0 }
        return Double(b.frameLength) / b.format.sampleRate
    }

    /// The buffer that plays now.
    private var active: AVAudioPCMBuffer? { playsProcessed ? processed ?? original : original }

    /// Read the whole file (≈ 3 MB for 8 s stereo float). No-op if already loaded.
    public func load(_ url: URL) throws {
        if loadedURL == url { return }
        stop()
        let b = try Self.read(url)
        engine.disconnectNodeOutput(node)
        engine.connect(node, to: engine.mainMixerNode, format: b.format)
        original = b
        processed = nil
        hasProcessed = false
        loadedURL = url
    }

    public func loadProcessed(_ audio: DecodedAudio?) throws {
        let wasActive = isPlaying && playsProcessed && processed != nil
        guard let audio else {
            let resumeAt = wasActive ? currentFrame : nil
            processed = nil
            hasProcessed = false
            if let resumeAt { restart(at: resumeAt) }
            return
        }
        guard let o = original, Self.matches(audio, o), let b = AVAudioPCMBuffer(pcmFormat: o.format, frameCapacity: o.frameLength),
            let dst = b.floatChannelData
        else { throw ClipPlayerError.mismatchedRender }
        for (c, samples) in audio.channels.enumerated() {
            samples.withUnsafeBufferPointer { dst[c].update(from: $0.baseAddress!, count: samples.count) }
        }
        b.frameLength = o.frameLength
        let resumeAt = isPlaying && playsProcessed ? currentFrame : nil
        processed = b
        hasProcessed = true
        if let resumeAt { restart(at: resumeAt) }
    }

    public func setPlaysProcessed(_ on: Bool) {
        guard on != playsProcessed else { return }
        let resumeAt = isPlaying && processed != nil ? currentFrame : nil
        playsProcessed = on
        if let resumeAt { restart(at: resumeAt) }
    }

    public func play(from start: Double, to end: Double? = nil, loop: Bool = false) throws {
        guard let full = active else { return }
        stop()
        let sr = full.format.sampleRate
        let total = Int(full.frameLength)
        let s0 = min(max(0, Int((start * sr).rounded())), max(total - 1, 0))
        let s1 = min(total, max(s0 + 1, Int(((end ?? duration) * sr).rounded())))
        spanFrames = s0..<s1
        span = Double(s0) / sr...Double(s1) / sr
        looping = loop
        if !engine.isRunning {
            engine.prepare()
            try engine.start()
        }
        schedule(Self.plan(span: spanFrames, at: s0, loop: loop))
    }

    public func stop() {
        generation += 1
        node.stop()
        if engine.isRunning { engine.stop() }
        isPlaying = false
    }

    /// Where playback is now (seconds in the clip), nil when stopped.
    public var currentTime: Double? {
        guard isPlaying, let f = currentFrame, let sr = active?.format.sampleRate else { return nil }
        return Double(f) / sr
    }

    /// The clip frame being rendered now (ahead of what is heard by the output latency, ~10–20 ms;
    /// an A/B switch skips that much, which is fine for listening).
    private var currentFrame: Int? {
        guard isPlaying, !spanFrames.isEmpty, let t = node.lastRenderTime, let pt = node.playerTime(forNodeTime: t) else {
            return nil
        }
        return Self.position(span: spanFrames, offset: startOffset, played: Int(max(0, pt.sampleTime)), loop: looping)
    }

    /// Continue the current span from `frame` with whichever buffer is active now.
    private func restart(at frame: Int) {
        generation += 1
        node.stop()
        schedule(Self.plan(span: spanFrames, at: frame, loop: looping))
    }

    private func schedule(_ plan: PlaybackPlan) {
        guard let full = active else { return }
        generation += 1
        let token = generation
        startOffset = plan.offset
        if let first = plan.first, let seg = Self.slice(full, first) {
            if plan.loop == nil {
                node.scheduleBuffer(seg, at: nil, options: [], completionCallbackType: .dataPlayedBack) { [weak self] _ in
                    Task { @MainActor in self?.finished(token) }
                }
            } else {
                node.scheduleBuffer(seg, at: nil, options: [])
            }
        }
        if let loop = plan.loop, let seg = Self.slice(full, loop) {
            node.scheduleBuffer(seg, at: nil, options: .loops)
        }
        node.play()
        isPlaying = true
    }

    private func finished(_ token: Int) {
        guard token == generation else { return }
        stop()
    }

    // MARK: Scheduling maths (pure; tested)

    /// What to schedule to play `span` starting at `frame`: the rest of the span once, then (when
    /// looping) the whole span on repeat. `offset` = where in the span playback starts.
    struct PlaybackPlan: Equatable, Sendable {
        var first: Range<Int>?
        var loop: Range<Int>?
        var offset: Int
    }

    nonisolated static func plan(span: Range<Int>, at frame: Int, loop: Bool) -> PlaybackPlan {
        guard !span.isEmpty else { return PlaybackPlan(first: nil, loop: nil, offset: 0) }
        let f = min(max(frame, span.lowerBound), span.upperBound - 1)
        let offset = f - span.lowerBound
        if !loop { return PlaybackPlan(first: f..<span.upperBound, loop: nil, offset: offset) }
        return PlaybackPlan(first: offset > 0 ? f..<span.upperBound : nil, loop: span, offset: offset)
    }

    /// The clip frame after `played` frames of a schedule that started `offset` into `span`.
    nonisolated static func position(span: Range<Int>, offset: Int, played: Int, loop: Bool) -> Int {
        let n = span.count
        guard n > 0 else { return span.lowerBound }
        let p = offset + max(0, played)
        return span.lowerBound + (loop ? p % n : min(p, n))
    }

    // MARK: Buffers

    public enum ClipPlayerError: Error, LocalizedError {
        case mismatchedRender
        public var errorDescription: String? { "the processed clip doesn't line up with the original" }
    }

    /// Same rate, channel count and length as the open clip (so A/B lines up sample for sample).
    nonisolated static func matches(_ audio: DecodedAudio, _ buffer: AVAudioPCMBuffer) -> Bool {
        audio.sampleRate == buffer.format.sampleRate && audio.channels.count == Int(buffer.format.channelCount)
            && audio.channels.allSatisfy { $0.count == Int(buffer.frameLength) } && audio.frames > 0
    }

    private static func read(_ url: URL) throws -> AVAudioPCMBuffer {
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let frames = AVAudioFrameCount(file.length)
        guard let b = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: max(frames, 1)) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        if frames > 0 { try file.read(into: b, frameCount: frames) }
        return b
    }

    private static func slice(_ src: AVAudioPCMBuffer, _ range: Range<Int>) -> AVAudioPCMBuffer? {
        let n = range.count
        guard n > 0, range.upperBound <= Int(src.frameLength), let src32 = src.floatChannelData,
            let out = AVAudioPCMBuffer(pcmFormat: src.format, frameCapacity: AVAudioFrameCount(n)),
            let dst = out.floatChannelData
        else { return nil }
        for c in 0..<Int(src.format.channelCount) {
            dst[c].update(from: src32[c] + range.lowerBound, count: n)
        }
        out.frameLength = AVAudioFrameCount(n)
        return out
    }
}
