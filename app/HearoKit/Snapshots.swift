import AVFoundation
import Accelerate
import Foundation

// Snapshot review model layer (M6): listing, sidecar round-trip, WAV decoding, peaks, and the
// region-editing operations behind the review window. No AppKit/SwiftUI here so it unit-tests
// cleanly. See docs/DESIGN.md "Snapshot review & annotation window".

/// What a region on a clip is. Raw values are the sidecar strings (same spelling as the
/// `hearo dataset` proposals: `lip_smack`, `mouth_click`, ...). Keyboard 1–6 in this order.
public enum RegionClass: String, CaseIterable, Sendable, Identifiable {
    case lipSmack = "lip_smack"
    case mouthClick = "mouth_click"
    case plosive
    case breath
    case filledPause = "filled_pause"
    case other

    public var id: String { rawValue }

    /// The 1–6 shortcut.
    public var key: Int { (Self.allCases.firstIndex(of: self) ?? 0) + 1 }

    public init?(key: Int) {
        guard (1...Self.allCases.count).contains(key) else { return nil }
        self = Self.allCases[key - 1]
    }

    /// Lenient parse for proposal/sidecar strings; anything unknown (e.g. `sibilance`) is `other`.
    public init(sidecar s: String?) {
        self = s.flatMap(RegionClass.init(rawValue:)) ?? .other
    }

    public var title: String {
        switch self {
        case .lipSmack: "Lip smack"
        case .mouthClick: "Mouth click"
        case .plosive: "Plosive"
        case .breath: "Breath"
        case .filledPause: "Filled pause"
        case .other: "Other"
        }
    }
}

/// A human-labelled span of a clip (seconds from the clip start).
public struct Region: Identifiable, Equatable, Sendable {
    public let id: UUID
    public var start: Double
    public var end: Double
    public var cls: RegionClass

    public init(id: UUID = UUID(), start: Double, end: Double, cls: RegionClass = .lipSmack) {
        self.id = id
        self.start = start
        self.end = end
        self.cls = cls
    }

    public var length: Double { end - start }
}

/// What the offline detector suggested (`hearo dataset propose`); read-only here.
public struct Proposal: Equatable, Sendable, Identifiable {
    public var id: Int  // index in the sidecar array
    public var start: Double
    public var end: Double
    public var cls: RegionClass
    public var classString: String
    public var module: String
    public var dbRemoved: Double?
    public var inPressWindow: Bool
}

/// A detector proposal the human marked as a false positive (sidecar `rejected`): what the
/// detector got wrong, kept as a hard negative for training (ADR 0018).
public struct RejectedSpan: Equatable, Sendable {
    public var start: Double
    public var end: Double
    public var classString: String
    public var module: String

    public init(start: Double, end: Double, classString: String, module: String) {
        self.start = start
        self.end = end
        self.classString = classString
        self.module = module
    }
}

/// What the human decided about a proposal.
public enum ProposalDecision: Equatable, Sendable {
    case undecided
    /// It is what the detector said (a region with its span exists).
    case accepted
    /// False positive (in `rejected`).
    case rejected
}

/// One snapshot on disk, as shown in the list.
public struct SnapshotInfo: Identifiable, Equatable, Sendable {
    public var id: String { stem }
    public var stem: String
    public var wavURL: URL
    public var jsonURL: URL
    public var date: Date
    /// "missed" / "false_hit" (or whatever the sidecar says).
    public var label: String
    public var reviewed: Bool
    public var proposalCount: Int
    public var regionCount: Int

    public var labelTitle: String {
        switch label {
        case "missed": "Missed"
        case "false_hit": "False hit"
        case "auto": "Auto"
        default: label
        }
    }
}

/// Min/max of the samples under one pixel column.
public struct PeakColumn: Equatable, Sendable {
    public var min: Float
    public var max: Float
}

/// A decoded clip: mono mix for display (the player uses the original file).
public struct AudioClip: Sendable, Equatable {
    public let mono: [Float]
    public let sampleRate: Double
    public let channels: Int
    /// Largest absolute sample (for display normalisation).
    public let peakAbs: Float

    public init(mono: [Float], sampleRate: Double, channels: Int = 1) {
        self.mono = mono
        self.sampleRate = sampleRate
        self.channels = channels
        var m: Float = 0
        if !mono.isEmpty { vDSP_maxmgv(mono, 1, &m, vDSP_Length(mono.count)) }
        self.peakAbs = m
    }

    public var duration: Double { Double(mono.count) / sampleRate }

    /// Min/max per column for the time span `[start, end)` split into `columns` equal columns.
    /// Every column covers at least one sample, so deep zoom shows steps, not gaps; columns
    /// outside the clip are `(0, 0)`.
    public func peaks(from start: Double, to end: Double, columns: Int) -> [PeakColumn] {
        guard columns > 0, end > start, !mono.isEmpty else { return [] }
        let n = mono.count
        let spc = (end - start) * sampleRate / Double(columns)  // samples per column
        let first = start * sampleRate
        var out = [PeakColumn](repeating: PeakColumn(min: 0, max: 0), count: columns)
        mono.withUnsafeBufferPointer { buf in
            guard let base = buf.baseAddress else { return }
            for i in 0..<columns {
                var s0 = Int((first + Double(i) * spc).rounded(.down))
                var s1 = Int((first + Double(i + 1) * spc).rounded(.down))
                if s1 <= s0 { s1 = s0 + 1 }
                s0 = Swift.max(s0, 0)
                s1 = Swift.min(s1, n)
                guard s1 > s0 else { continue }
                var lo: Float = 0
                var hi: Float = 0
                vDSP_minv(base + s0, 1, &lo, vDSP_Length(s1 - s0))
                vDSP_maxv(base + s0, 1, &hi, vDSP_Length(s1 - s0))
                out[i] = PeakColumn(min: lo, max: hi)
            }
        }
        return out
    }

    /// A smack-like synthetic clip for previews: quiet "speech" (noise plus a voiced harmonic
    /// series), 50 Hz hum, an "s" (noise around 7 kHz) and a couple of clicks — so the
    /// waveform and the spectrogram both have something to show.
    public static func synthetic(seconds: Double = 8, sampleRate: Double = 48_000) -> AudioClip {
        let n = Int(seconds * sampleRate)
        var x = [Float](repeating: 0, count: n)
        var seed: UInt64 = 0x9E37_79B9_7F4A_7C15
        var smooth: Float = 0
        for i in 0..<n {
            seed = seed &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            let noise = Float(Int64(bitPattern: seed >> 11) % 2000) / 2000 - 0.5
            smooth += 0.1 * (noise - smooth)  // ~800 Hz low-pass
            let t = Double(i) / sampleRate
            let envelope = Float(0.5 + 0.5 * sin(2 * .pi * 0.7 * t)) * 0.25
            // Voiced: ~130 Hz with a slow glide, harmonics falling 1/h.
            let phase = 2 * .pi * (130 * t + 4 * sin(2 * .pi * 0.3 * t))
            var voiced = 0.0
            for h in 1...16 { voiced += sin(Double(h) * phase) / Double(h) }
            let hum = Float(0.004 * sin(2 * .pi * 50 * t))
            // "s": low-passed noise shifted up to ~7 kHz.
            let ess = (2.2..<2.45).contains(t) ? Float(sin(2 * .pi * 7000 * t)) * smooth * 0.6 : 0
            x[i] = noise * envelope * 0.15 + Float(voiced * 0.12) * envelope + hum + ess
        }
        for at in [5.9, 6.6] {
            let s = Int(at * sampleRate)
            for k in 0..<Int(0.012 * sampleRate) where s + k < n {
                let decay = exp(-Double(k) / (0.002 * sampleRate))
                x[s + k] += Float(0.8 * decay * (k == 0 ? 1 : sin(2 * .pi * 2500 * Double(k) / sampleRate)))
            }
        }
        return AudioClip(mono: x, sampleRate: sampleRate)
    }
}

/// What a point on the waveform hits.
public enum RegionHit: Equatable, Sendable {
    case none
    case inside(UUID)
    case startEdge(UUID)
    case endEdge(UUID)
}

/// The editable annotation of one clip, decoded from (and merged back into) its sidecar.
public struct Annotation: Equatable, Sendable {
    public var duration: Double
    public var sampleRate: Double
    /// Where the event most likely is (shaded); `event_hint_s` or the old-file default.
    public var hint: ClosedRange<Double>
    public var pressAt: Double?
    public var proposals: [Proposal]
    public var regions: [Region]
    /// Proposals marked as false positives.
    public var rejected: [RejectedSpan]
    public var reviewed: Bool

    public init(
        duration: Double, sampleRate: Double, hint: ClosedRange<Double>? = nil, pressAt: Double? = nil,
        proposals: [Proposal] = [], regions: [Region] = [], rejected: [RejectedSpan] = [], reviewed: Bool = false
    ) {
        self.duration = duration
        self.sampleRate = sampleRate
        self.hint = hint ?? Self.defaultHint(duration)
        self.pressAt = pressAt
        self.proposals = proposals
        self.regions = regions
        self.rejected = rejected
        self.reviewed = reviewed
    }

    /// Old snapshots have no `event_hint_s`: 2.5…0.3 s before the end (the press).
    public static func defaultHint(_ seconds: Double) -> ClosedRange<Double> {
        max(0, seconds - 2.5)...max(0, seconds - 0.3)
    }

    /// Decode from a sidecar. `duration`/`sampleRate` come from the audio when known (the
    /// sidecar's `seconds`/`sample_rate` are the fallback).
    public init(sidecar: [String: Any], duration: Double? = nil, sampleRate: Double? = nil) {
        let secs = duration ?? Self.double(sidecar["seconds"]) ?? 8
        let sr = sampleRate ?? Self.double(sidecar["sample_rate"]) ?? 48_000
        var hint: ClosedRange<Double>?
        if let h = sidecar["event_hint_s"] as? [Any], h.count == 2, let a = Self.double(h[0]), let b = Self.double(h[1]) {
            hint = min(a, b)...max(a, b)
        }
        let proposals = (sidecar["proposals"] as? [[String: Any]] ?? []).enumerated().compactMap { i, p -> Proposal? in
            guard let s = Self.double(p["start_s"]), let e = Self.double(p["end_s"]) else { return nil }
            let cls = p["class"] as? String
            return Proposal(
                id: i, start: min(s, e), end: max(s, e), cls: RegionClass(sidecar: cls), classString: cls ?? "other",
                module: p["module"] as? String ?? "", dbRemoved: Self.double(p["db_removed"]),
                inPressWindow: p["in_press_window"] as? Bool ?? false)
        }
        let regions = (sidecar["regions"] as? [[String: Any]] ?? []).compactMap { r -> Region? in
            guard let s = Self.double(r["start_s"]), let e = Self.double(r["end_s"]) else { return nil }
            return Region(start: min(s, e), end: max(s, e), cls: RegionClass(sidecar: r["class"] as? String))
        }
        let rejected = (sidecar["rejected"] as? [[String: Any]] ?? []).compactMap { r -> RejectedSpan? in
            guard let s = Self.double(r["start_s"]), let e = Self.double(r["end_s"]) else { return nil }
            return RejectedSpan(
                start: min(s, e), end: max(s, e), classString: r["class"] as? String ?? "other",
                module: r["module"] as? String ?? "")
        }
        self.init(
            duration: secs, sampleRate: sr, hint: hint ?? Self.defaultHint(Self.double(sidecar["seconds"]) ?? secs),
            pressAt: Self.double(sidecar["press_at_s"]), proposals: proposals, regions: regions, rejected: rejected,
            reviewed: sidecar["reviewed"] as? Bool ?? false)
    }

    /// The sidecar with our keys (`regions`, `rejected`, `reviewed`) replaced and everything else
    /// untouched.
    public func merged(into sidecar: [String: Any]) -> [String: Any] {
        var out = sidecar
        out["regions"] = regions.sorted { $0.start < $1.start }.map { r -> [String: Any] in
            ["start_s": Self.round6(r.start), "end_s": Self.round6(r.end), "class": r.cls.rawValue]
        }
        if !rejected.isEmpty || sidecar["rejected"] != nil {
            out["rejected"] = rejected.sorted { $0.start < $1.start }.map { r -> [String: Any] in
                ["start_s": Self.round6(r.start), "end_s": Self.round6(r.end), "class": r.classString, "module": r.module]
            }
        }
        if reviewed || sidecar["reviewed"] != nil { out["reviewed"] = reviewed }
        return out
    }

    // MARK: Editing

    /// Snap to the sample grid and clamp to the clip.
    public func snap(_ t: Double) -> Double {
        min(duration, max(0, (t * sampleRate).rounded() / sampleRate))
    }

    private var oneSample: Double { 1 / sampleRate }

    public func region(_ id: UUID) -> Region? { regions.first { $0.id == id } }

    /// New region over the dragged range (either direction). Nil if it is shorter than a sample.
    @discardableResult
    public mutating func addRegion(from a: Double, to b: Double, cls: RegionClass = .lipSmack) -> UUID? {
        let s = snap(min(a, b))
        let e = snap(max(a, b))
        guard e - s >= oneSample * 0.5 else { return nil }
        let r = Region(start: s, end: e, cls: cls)
        regions.append(r)
        return r.id
    }

    /// Set both edges (any order), clamped and snapped; keeps at least one sample.
    public mutating func setBounds(_ id: UUID, _ a: Double, _ b: Double) {
        guard let i = regions.firstIndex(where: { $0.id == id }) else { return }
        var s = snap(min(a, b))
        var e = snap(max(a, b))
        if e - s < oneSample * 0.5 {
            if e + oneSample <= duration { e = snap(s + oneSample) } else { s = snap(e - oneSample) }
        }
        regions[i].start = s
        regions[i].end = e
    }

    /// Move so it starts at `start`, keeping its length, clamped inside the clip.
    public mutating func move(_ id: UUID, toStart start: Double) {
        guard let i = regions.firstIndex(where: { $0.id == id }) else { return }
        let len = regions[i].length
        let s = min(max(0, start), max(0, duration - len))
        let snapped = snap(s)
        regions[i].start = snapped
        regions[i].end = min(duration, snapped + len)
    }

    public enum Edge: Sendable { case start, end }

    /// Drag one edge to `t`; it stops one sample short of the other edge.
    public mutating func resize(_ id: UUID, edge: Edge, to t: Double) {
        guard let i = regions.firstIndex(where: { $0.id == id }) else { return }
        switch edge {
        case .start: regions[i].start = min(snap(t), snap(regions[i].end - oneSample))
        case .end: regions[i].end = max(snap(t), snap(regions[i].start + oneSample))
        }
    }

    public mutating func delete(_ id: UUID) { regions.removeAll { $0.id == id } }

    public mutating func setClass(_ id: UUID, _ cls: RegionClass) {
        guard let i = regions.firstIndex(where: { $0.id == id }) else { return }
        regions[i].cls = cls
    }

    /// Turn a proposal into a region (same span and class). Accepting twice is a no-op.
    @discardableResult
    public mutating func accept(proposal index: Int) -> UUID? {
        guard let p = proposals.first(where: { $0.id == index }) else { return nil }
        let tol = matchTolerance
        rejected.removeAll { Self.matches($0, p, tol) }
        if let existing = acceptedRegion(for: p) { return existing.id }
        return addRegion(from: p.start, to: p.end, cls: p.cls)
    }

    /// The region that matches a proposal's span (within half a sample), if accepted.
    public func acceptedRegion(for p: Proposal) -> Region? {
        let tol = oneSample * 0.5 + 1e-6
        return regions.first { abs($0.start - snap(p.start)) <= tol && abs($0.end - snap(p.end)) <= tol }
    }

    private var matchTolerance: Double { oneSample * 0.5 + 1e-6 }

    private static func matches(_ r: RejectedSpan, _ p: Proposal, _ tol: Double) -> Bool {
        abs(r.start - p.start) <= tol && abs(r.end - p.end) <= tol
    }

    /// Accepted (a region with its span), rejected (a false positive), or neither.
    public func decision(for p: Proposal) -> ProposalDecision {
        if acceptedRegion(for: p) != nil { return .accepted }
        let tol = matchTolerance
        return rejected.contains { Self.matches($0, p, tol) } ? .rejected : .undecided
    }

    /// The first proposal (in sidecar order) nobody decided on yet.
    public var firstUndecided: Int? { proposals.first { decision(for: $0) == .undecided }?.id }

    /// Mark a proposal as a false positive: drop its accepted region, if any, and record its span
    /// in `rejected` (once).
    public mutating func reject(proposal index: Int) {
        guard let p = proposals.first(where: { $0.id == index }) else { return }
        if let r = acceptedRegion(for: p) { regions.removeAll { $0.id == r.id } }
        let tol = matchTolerance
        if !rejected.contains(where: { Self.matches($0, p, tol) }) {
            rejected.append(RejectedSpan(start: p.start, end: p.end, classString: p.classString, module: p.module))
        }
    }

    /// Edges win over insides (within `tolerance` seconds); the smallest region wins on overlap.
    public func hit(at t: Double, tolerance: Double) -> RegionHit {
        let byLength = regions.sorted { $0.length < $1.length }
        for r in byLength {
            let edgeTol = min(tolerance, r.length / 3)
            if abs(t - r.start) <= edgeTol { return .startEdge(r.id) }
            if abs(t - r.end) <= edgeTol { return .endEdge(r.id) }
        }
        if let r = byLength.first(where: { t >= $0.start && t <= $0.end }) { return .inside(r.id) }
        return .none
    }

    static func double(_ v: Any?) -> Double? {
        switch v {
        case let n as NSNumber: n.doubleValue
        case let d as Double: d
        case let i as Int: Double(i)
        default: nil
        }
    }

    static func round6(_ x: Double) -> Double { (x * 1_000_000).rounded() / 1_000_000 }
}

/// The visible part of a clip (zoom + pan).
public struct Viewport: Equatable, Sendable {
    public private(set) var start: Double
    public private(set) var length: Double
    public let total: Double
    /// Deepest zoom: 2 ms across the view (smacks are 5–30 ms).
    public static let minLength = 0.002

    public init(total: Double) {
        self.total = max(total, Self.minLength)
        self.start = 0
        self.length = self.total
    }

    public var end: Double { start + length }

    public func time(atX x: Double, width: Double) -> Double { start + x / max(width, 1) * length }
    public func x(at t: Double, width: Double) -> Double { (t - start) / length * width }

    /// `factor > 1` zooms in, keeping `anchor` (a time) under the same pixel.
    public mutating func zoom(by factor: Double, around anchor: Double) {
        guard factor > 0, factor.isFinite else { return }
        let frac = (anchor - start) / length
        length = min(total, max(Self.minLength, length / factor))
        start = anchor - frac * length
        clamp()
    }

    public mutating func pan(by seconds: Double) {
        start += seconds
        clamp()
    }

    /// Show `range` with `padding` (fraction of its length) on each side.
    public mutating func show(_ range: ClosedRange<Double>, padding: Double = 0.5) {
        let len = range.upperBound - range.lowerBound
        length = min(total, max(Self.minLength, len * (1 + 2 * padding)))
        start = (range.lowerBound + range.upperBound) / 2 - length / 2
        clamp()
    }

    public mutating func reset() {
        start = 0
        length = total
    }

    /// Where the view should be to keep a moving playhead at `t` visible: when zoomed in and
    /// `t` has left the view, the next "page" (same zoom, `t` just inside the left edge);
    /// otherwise nil (nothing to change).
    public func following(_ t: Double) -> Viewport? {
        guard length < total, t < start || t > end else { return nil }
        var v = self
        v.pan(by: t - length * 0.05 - start)
        return v
    }

    private mutating func clamp() {
        start = min(max(0, start), total - length)
    }
}

/// The snapshot folder: `snapshot-<unix>-<label>[-n].wav` + `.json` sidecars.
public struct SnapshotLibrary: Sendable {
    public let folder: URL
    /// How a deleted clip's files go away: to the Trash (recoverable) in the app.
    private var remove: @Sendable (URL) throws -> Void = { try FileManager.default.trashItem(at: $0, resultingItemURL: nil) }

    /// Record of deleted clips, shared with the Rust importer (`hearo_engine::snapshots::DELETED_FILE`):
    /// one `<stem>\t<wav bytes>` line each, so importing another Mac's export doesn't bring them back.
    public static let deletedFile = "deleted.txt"

    public init(folder: URL) { self.folder = folder }

    /// The same library, removing files with `remover` instead of the Trash (tests).
    public func withRemover(_ remover: @escaping @Sendable (URL) throws -> Void) -> SnapshotLibrary {
        var copy = self
        copy.remove = remover
        return copy
    }

    /// Move a clip (wav + sidecar) to the Trash and record it in `deleted.txt`.
    public func delete(_ info: SnapshotInfo) throws {
        let size = (try? FileManager.default.attributesOfItem(atPath: info.wavURL.path)[.size] as? Int) ?? nil
        try remove(info.wavURL)
        if FileManager.default.fileExists(atPath: info.jsonURL.path) { try remove(info.jsonURL) }
        guard let size else { return }
        let record = folder.appendingPathComponent(Self.deletedFile)
        let line = Data("\(info.stem)\t\(size)\n".utf8)
        if let handle = try? FileHandle(forWritingTo: record) {
            defer { try? handle.close() }
            try handle.seekToEnd()
            try handle.write(contentsOf: line)
        } else {
            let header = "# Snapshots deleted in hEARo's review window; import skips these (stem<TAB>wav bytes).\n"
            try (Data(header.utf8) + line).write(to: record)
        }
    }

    /// All snapshots, newest first. A missing folder is an empty list.
    public func list() -> [SnapshotInfo] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
        return names.compactMap { name -> SnapshotInfo? in
            guard name.hasPrefix("snapshot-"), name.hasSuffix(".wav") else { return nil }
            let stem = String(name.dropLast(4))
            let wav = folder.appendingPathComponent(name)
            let json = folder.appendingPathComponent(stem + ".json")
            let side = (try? Self.readSidecar(json)) ?? [:]
            let parts = stem.split(separator: "-")
            let fileUnix = parts.count > 1 ? Double(parts[1]) : nil
            let unix = Annotation.double(side["unix_time"]) ?? fileUnix ?? 0
            let label = side["label"] as? String ?? (parts.count > 2 ? String(parts[2]) : "?")
            return SnapshotInfo(
                stem: stem, wavURL: wav, jsonURL: json, date: Date(timeIntervalSince1970: unix), label: label,
                reviewed: side["reviewed"] as? Bool ?? false,
                proposalCount: (side["proposals"] as? [Any])?.count ?? 0,
                regionCount: (side["regions"] as? [Any])?.count ?? 0)
        }
        .sorted { ($0.date, $0.stem) > ($1.date, $1.stem) }
    }

    /// The sidecar as a plain dictionary (missing file → empty).
    public static func readSidecar(_ url: URL) throws -> [String: Any] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        let data = try Data(contentsOf: url)
        return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    }

    /// Write `annotation` into the sidecar at `url`, preserving every other key. Re-reads the
    /// file first so keys written meanwhile (e.g. new `proposals`) are not lost.
    public static func save(_ annotation: Annotation, to url: URL) throws {
        let merged = annotation.merged(into: try readSidecar(url))
        let data = try JSONSerialization.data(withJSONObject: merged, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
        try data.write(to: url, options: .atomic)
    }

    /// Decode a WAV into a mono mix (average of channels) of 32-bit floats.
    public static func loadAudio(_ url: URL) throws -> AudioClip {
        let file = try AVAudioFile(forReading: url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let frames = AVAudioFrameCount(file.length)
        guard let buf = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: max(frames, 1)) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        if frames > 0 { try file.read(into: buf, frameCount: frames) }
        let n = Int(buf.frameLength)
        let ch = Int(buf.format.channelCount)
        var mono = [Float](repeating: 0, count: n)
        if let data = buf.floatChannelData, n > 0, ch > 0 {
            for c in 0..<ch {
                vDSP_vadd(mono, 1, data[c], 1, &mono, 1, vDSP_Length(n))
            }
            var scale = 1 / Float(ch)
            vDSP_vsmul(mono, 1, &scale, &mono, 1, vDSP_Length(n))
        }
        return AudioClip(mono: mono, sampleRate: file.processingFormat.sampleRate, channels: ch)
    }
}
