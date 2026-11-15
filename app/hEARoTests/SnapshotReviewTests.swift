import AVFoundation
import Foundation
import Testing

@testable import HearoKit

/// `#require` for mutating calls (the macro can't wrap a mutating member call).
func req<T>(_ value: T?, sourceLocation: SourceLocation = #_sourceLocation) throws -> T {
    try #require(value, sourceLocation: sourceLocation)
}

/// Temp snapshot folder with WAV + JSON fixtures, removed on deinit.
final class Fixture {
    let dir: URL

    init() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("hearo-review-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    deinit { try? FileManager.default.removeItem(at: dir) }

    /// Stereo 32-bit float WAV; left = `left(i)`, right = `right(i)`.
    @discardableResult
    func wav(
        _ stem: String, seconds: Double = 1, sampleRate: Double = 48_000, left: (Int) -> Float = { _ in 0 },
        right: ((Int) -> Float)? = nil
    ) throws -> URL {
        let url = dir.appendingPathComponent(stem + ".wav")
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: sampleRate, AVNumberOfChannelsKey: 2,
            AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true, AVLinearPCMIsNonInterleaved: false,
        ]
        let file = try AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        let n = Int(seconds * sampleRate)
        let buf = try #require(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(n)))
        buf.frameLength = AVAudioFrameCount(n)
        let ch = try #require(buf.floatChannelData)
        for i in 0..<n {
            ch[0][i] = left(i)
            ch[1][i] = right?(i) ?? left(i)
        }
        try file.write(from: buf)
        file.close()
        return url
    }

    @discardableResult
    func json(_ stem: String, _ object: [String: Any]) throws -> URL {
        let url = dir.appendingPathComponent(stem + ".json")
        try JSONSerialization.data(withJSONObject: object).write(to: url)
        return url
    }

    func read(_ stem: String) throws -> [String: Any] {
        try SnapshotLibrary.readSidecar(dir.appendingPathComponent(stem + ".json"))
    }
}

struct SnapshotLibraryTests {
    @Test func listsNewestFirstWithSidecarFacts() throws {
        let f = try Fixture()
        try f.wav("snapshot-100-missed")
        try f.json("snapshot-100-missed", ["label": "missed", "unix_time": 100, "reviewed": true, "regions": [["start_s": 0.1, "end_s": 0.2, "class": "plosive"]]])
        try f.wav("snapshot-300-false_hit")
        try f.json("snapshot-300-false_hit", ["label": "false_hit", "unix_time": 300, "proposals": [["start_s": 0.1, "end_s": 0.12]]])
        try f.wav("snapshot-200-missed-1")  // no sidecar: facts from the file name
        try "not audio".write(to: f.dir.appendingPathComponent("notes.txt"), atomically: true, encoding: .utf8)

        let clips = SnapshotLibrary(folder: f.dir).list()
        #expect(clips.map(\.stem) == ["snapshot-300-false_hit", "snapshot-200-missed-1", "snapshot-100-missed"])
        #expect(clips[0].labelTitle == "False hit")
        #expect(clips[0].proposalCount == 1)
        #expect(!clips[0].reviewed)
        #expect(clips[1].label == "missed")
        #expect(clips[1].date == Date(timeIntervalSince1970: 200))
        #expect(clips[2].reviewed)
        #expect(clips[2].regionCount == 1)
    }

    @Test func missingFolderIsEmpty() {
        #expect(SnapshotLibrary(folder: URL(fileURLWithPath: "/nonexistent/hearo-\(UUID())")).list().isEmpty)
    }

    @Test func decodesStereoWavToMonoMix() throws {
        let f = try Fixture()
        let url = try f.wav("snapshot-1-missed", seconds: 0.5, sampleRate: 44_100, left: { _ in 0.5 }, right: { i in i == 10 ? -1 : 0.1 })
        let clip = try SnapshotLibrary.loadAudio(url)
        #expect(clip.sampleRate == 44_100)
        #expect(clip.channels == 2)
        #expect(clip.mono.count == 22_050)
        #expect(abs(clip.duration - 0.5) < 1e-9)
        #expect(abs(clip.mono[0] - 0.3) < 1e-6)
        #expect(abs(clip.mono[10] - -0.25) < 1e-6)
        #expect(abs(clip.peakAbs - 0.3) < 1e-6)
    }
}

struct PeakTests {
    let clip = AudioClip(mono: (0..<1000).map { Float($0 % 10) / 10 - 0.45 }, sampleRate: 1000)  // 1 s

    @Test func minMaxPerColumnAtFullView() {
        let p = clip.peaks(from: 0, to: 1, columns: 100)  // 10 samples per column = one sawtooth period
        #expect(p.count == 100)
        #expect(p.allSatisfy { abs($0.min - -0.45) < 1e-6 && abs($0.max - 0.45) < 1e-6 })
    }

    @Test func deepZoomCoversAtLeastOneSample() {
        let p = clip.peaks(from: 0.0, to: 0.004, columns: 40)  // 4 samples on 40 columns
        #expect(p.count == 40)
        #expect(p[0] == PeakColumn(min: -0.45, max: -0.45))
        #expect(abs(p[39].max - -0.15) < 1e-6)  // sample 3
    }

    @Test func outsideTheClipIsSilent() {
        let p = clip.peaks(from: 0.9, to: 1.1, columns: 20)
        #expect(p[19] == PeakColumn(min: 0, max: 0))
        #expect(p[0].max > 0)
    }

    @Test func degenerateRequests() {
        #expect(clip.peaks(from: 0, to: 1, columns: 0).isEmpty)
        #expect(clip.peaks(from: 1, to: 1, columns: 10).isEmpty)
    }
}

struct AnnotationTests {
    let sr = 48_000.0

    func blank(_ secs: Double = 8) -> Annotation { Annotation(duration: secs, sampleRate: sr) }

    @Test func decodesSidecarAndFallsBackForOldFiles() throws {
        let a = Annotation(sidecar: [
            "seconds": 8.0, "sample_rate": 48_000, "press_at_s": 8.0, "event_hint_s": [5.5, 7.7],
            "proposals": [["start_s": 6.6, "end_s": 6.612, "class": "lip_smack", "module": "declick", "db_removed": 9.5, "in_press_window": true],
                          ["start_s": 1.0, "end_s": 1.01, "class": "sibilance"]],
            "regions": [["start_s": 2.0, "end_s": 1.5, "class": "breath"]],
        ])
        #expect(a.hint == 5.5...7.7)
        #expect(a.pressAt == 8)
        #expect(a.proposals.count == 2)
        #expect(a.proposals[0].cls == .lipSmack && a.proposals[0].inPressWindow && a.proposals[0].dbRemoved == 9.5)
        #expect(a.proposals[1].cls == .other && a.proposals[1].classString == "sibilance")
        #expect(a.regions.count == 1 && a.regions[0].start == 1.5 && a.regions[0].cls == .breath)
        #expect(!a.reviewed)

        let old = Annotation(sidecar: ["seconds": 6.0], duration: 6, sampleRate: 44_100)
        #expect(old.hint == 3.5...5.7)
        #expect(old.proposals.isEmpty && old.regions.isEmpty)
    }

    @Test func classKeysAreOneToSix() {
        #expect(RegionClass.allCases.map(\.key) == [1, 2, 3, 4, 5, 6])
        #expect(RegionClass(key: 1) == .lipSmack)
        #expect(RegionClass(key: 5) == .filledPause)
        #expect(RegionClass(key: 0) == nil && RegionClass(key: 7) == nil)
        #expect(RegionClass.filledPause.rawValue == "filled_pause")
    }

    @Test func createByDragEitherDirectionSnapsToSamples() throws {
        var a = blank()
        let id = try req(a.addRegion(from: 6.01234567, to: 6.0))
        let r = try #require(a.region(id))
        #expect(r.start == 6.0)
        #expect(r.end == (6.01234567 * sr).rounded() / sr)
        #expect(r.cls == .lipSmack)
        #expect(a.addRegion(from: 1, to: 1) == nil)  // a click is not a region
        #expect(a.addRegion(from: -1, to: 0.5).map { a.region($0)!.start } == 0)  // clamped
    }

    @Test func moveKeepsLengthAndClamps() throws {
        var a = blank()
        let id = try req(a.addRegion(from: 1, to: 1.5))
        a.move(id, toStart: 3)
        #expect(a.region(id)!.start == 3 && a.region(id)!.end == 3.5)
        a.move(id, toStart: 7.9)
        #expect(a.region(id)!.end == 8 && a.region(id)!.start == 7.5)
        a.move(id, toStart: -2)
        #expect(a.region(id)!.start == 0 && a.region(id)!.end == 0.5)
    }

    @Test func resizeEdgesStopShortOfEachOther() throws {
        var a = blank()
        let id = try req(a.addRegion(from: 1, to: 2))
        a.resize(id, edge: .start, to: 0.5)
        a.resize(id, edge: .end, to: 2.25)
        #expect(a.region(id)!.start == 0.5 && a.region(id)!.end == 2.25)
        a.resize(id, edge: .start, to: 5)  // past the end edge
        #expect(abs(a.region(id)!.end - a.region(id)!.start - 1 / sr) < 1e-9)
        a.resize(id, edge: .end, to: 99)
        #expect(a.region(id)!.end == 8)
        a.setBounds(id, 4, 3)
        #expect(a.region(id)!.start == 3 && a.region(id)!.end == 4)
    }

    @Test func deleteAndSetClass() throws {
        var a = blank()
        let x = try req(a.addRegion(from: 1, to: 2))
        let y = try req(a.addRegion(from: 3, to: 4))
        a.setClass(y, .filledPause)
        #expect(a.region(y)!.cls == .filledPause)
        a.delete(x)
        #expect(a.regions.map(\.id) == [y])
    }

    @Test func acceptProposalOnce() throws {
        var a = Annotation(sidecar: ["proposals": [["start_s": 6.6, "end_s": 6.612, "class": "mouth_click"]]], duration: 8, sampleRate: sr)
        #expect(a.acceptedRegion(for: a.proposals[0]) == nil)
        let id = try req(a.accept(proposal: 0))
        #expect(a.region(id)!.cls == .mouthClick)
        #expect(abs(a.region(id)!.start - 6.6) < 1 / sr)
        #expect(a.acceptedRegion(for: a.proposals[0])?.id == id)
        #expect(a.accept(proposal: 0) == id)
        #expect(a.regions.count == 1)
        #expect(a.accept(proposal: 7) == nil)
    }

    /// Auto-saved catches are verified (accept → a region) or marked false positives (reject →
    /// `rejected`, the hard negatives); pressing the other key flips the decision.
    @Test func rejectAndAcceptFlipAProposalsDecision() throws {
        let props: [[String: Any]] = [
            ["start_s": 2.0, "end_s": 2.02, "class": "lip_smack", "module": "desmack"],
            ["start_s": 5.0, "end_s": 5.03, "class": "lip_smack", "module": "desmack"],
        ]
        var a = Annotation(sidecar: ["proposals": props], duration: 8, sampleRate: sr)
        #expect(a.decision(for: a.proposals[0]) == .undecided)
        #expect(a.firstUndecided == 0)
        a.reject(proposal: 0)
        #expect(a.decision(for: a.proposals[0]) == .rejected)
        #expect(a.regions.isEmpty && a.rejected.count == 1)
        #expect(a.firstUndecided == 1)
        a.reject(proposal: 0)
        #expect(a.rejected.count == 1, "rejecting twice is a no-op")
        _ = a.accept(proposal: 0)
        #expect(a.decision(for: a.proposals[0]) == .accepted && a.rejected.isEmpty)
        a.reject(proposal: 0)
        #expect(a.decision(for: a.proposals[0]) == .rejected && a.regions.isEmpty, "flipped back")
        _ = a.accept(proposal: 1)
        #expect(a.firstUndecided == nil)
        // Saved as `rejected` and read back.
        let out = a.merged(into: ["proposals": props])
        let rej = try #require(out["rejected"] as? [[String: Any]])
        #expect(rej.count == 1 && rej[0]["start_s"] as? Double == 2.0 && rej[0]["module"] as? String == "desmack")
        let back = Annotation(sidecar: out, duration: 8, sampleRate: sr)
        #expect(back.decision(for: back.proposals[0]) == .rejected)
        #expect(back.decision(for: back.proposals[1]) == .accepted)
        // Nothing rejected and never was: no key added.
        #expect(Annotation(duration: 8, sampleRate: sr).merged(into: [:])["rejected"] == nil)
    }

    /// Decisions still match after the sidecar round trip at 44.1 kHz, where proposal times (ms
    /// precision) aren't on the sample grid.
    @Test func decisionsSurviveTheSidecarRoundTripAt44k1() throws {
        let rate = 44_100.0
        let props: [[String: Any]] = [
            ["start_s": 4.98, "end_s": 5.0, "class": "lip_smack", "module": "desmack"],
            ["start_s": 6.123, "end_s": 6.141, "class": "lip_smack", "module": "desmack"],
        ]
        var a = Annotation(sidecar: ["proposals": props], duration: 8, sampleRate: rate)
        _ = a.accept(proposal: 0)
        a.reject(proposal: 1)
        let json = try JSONSerialization.data(withJSONObject: a.merged(into: ["proposals": props]))
        let side = try #require(try JSONSerialization.jsonObject(with: json) as? [String: Any])
        let back = Annotation(sidecar: side, duration: 8, sampleRate: rate)
        #expect(back.decision(for: back.proposals[0]) == .accepted)
        #expect(back.decision(for: back.proposals[1]) == .rejected)
        #expect(back.firstUndecided == nil && back.regions.count == 1 && back.rejected.count == 1)
    }

    @Test func hitTestingPrefersEdgesThenSmallestRegion() throws {
        var a = blank()
        let big = try req(a.addRegion(from: 1, to: 3))
        let small = try req(a.addRegion(from: 2, to: 2.02))
        #expect(a.hit(at: 1.001, tolerance: 0.005) == .startEdge(big))
        #expect(a.hit(at: 2.999, tolerance: 0.005) == .endEdge(big))
        #expect(a.hit(at: 1.5, tolerance: 0.005) == .inside(big))
        #expect(a.hit(at: 2.01, tolerance: 0.005) == .inside(small))  // edge zone shrinks to a third
        #expect(a.hit(at: 2.0005, tolerance: 0.005) == .startEdge(small))
        #expect(a.hit(at: 5, tolerance: 0.005) == .none)
    }

    @Test func mergePreservesUnknownKeys() throws {
        let side: [String: Any] = [
            "label": "missed", "unprocessed": true, "modules": ["declick", "deess"], "hearo_version": "0.1.0",
            "nested": ["a": 1], "proposals": [["start_s": 1.0, "end_s": 1.1, "extra": "keep"]],
        ]
        var a = Annotation(sidecar: side, duration: 8, sampleRate: sr)
        a.addRegion(from: 3, to: 3.5, cls: .plosive)
        a.addRegion(from: 1, to: 1.25)
        a.reviewed = true
        let out = a.merged(into: side)
        #expect(out["label"] as? String == "missed")
        #expect(out["unprocessed"] as? Bool == true)
        #expect(out["modules"] as? [String] == ["declick", "deess"])
        #expect((out["nested"] as? [String: Int]) == ["a": 1])
        #expect((out["proposals"] as? [[String: Any]])?.first?["extra"] as? String == "keep")
        #expect(out["reviewed"] as? Bool == true)
        let regions = try #require(out["regions"] as? [[String: Any]])
        #expect(regions.count == 2)
        #expect(regions[0]["start_s"] as? Double == 1 && regions[0]["class"] as? String == "lip_smack")  // sorted by start
        #expect(regions[1]["end_s"] as? Double == 3.5 && regions[1]["class"] as? String == "plosive")
        #expect(Set(regions[0].keys) == ["start_s", "end_s", "class"])
        // Not reviewed and never was: no key added.
        #expect(Annotation(duration: 8, sampleRate: sr).merged(into: [:])["reviewed"] == nil)
    }

    @Test func saveRoundTripsThroughDisk() throws {
        let f = try Fixture()
        let url = try f.json("snapshot-1-missed", ["label": "missed", "unprocessed": true, "seconds": 8, "custom": ["x": [1, 2]]])
        var a = Annotation(sidecar: try SnapshotLibrary.readSidecar(url), duration: 8, sampleRate: sr)
        a.addRegion(from: 2, to: 2.01, cls: .mouthClick)
        a.reviewed = true
        try SnapshotLibrary.save(a, to: url)

        let back = try SnapshotLibrary.readSidecar(url)
        #expect(back["unprocessed"] as? Bool == true)
        #expect(back["custom"] as? [String: [Int]] == ["x": [1, 2]])
        #expect(back["seconds"] as? Int == 8)
        let again = Annotation(sidecar: back, duration: 8, sampleRate: sr)
        #expect(again.reviewed)
        #expect(again.regions.count == 1 && again.regions[0].cls == .mouthClick && again.regions[0].start == 2)
        // Booleans stay JSON booleans (not 1/0) for the Rust/Python readers.
        let text = try String(contentsOf: url, encoding: .utf8)
        #expect(text.contains("\"reviewed\" : true"))
        #expect(text.contains("\"unprocessed\" : true"))
    }
}

struct ViewportTests {
    @Test func followingPagesWithThePlayheadOnlyWhenZoomedAndOutOfView() {
        #expect(Viewport(total: 8).following(7.9) == nil, "whole clip visible: never moves")
        var v = Viewport(total: 8)
        v.show(1...2, padding: 0)  // 1 s window at 1 s
        #expect(v.following(1.5) == nil, "inside the view: stays")
        let next = v.following(2.01)
        #expect(next.map { abs($0.start - 1.96) < 1e-9 && abs($0.length - 1) < 1e-12 } == true)
        #expect(v.following(7.99)?.end == 8, "clamped at the clip end")
        #expect(v.following(0.2).map { abs($0.start - 0.15) < 1e-9 } == true, "looping back to an earlier region jumps back")
    }

    @Test func zoomKeepsTheAnchorUnderThePointer() {
        var v = Viewport(total: 8)
        v.zoom(by: 4, around: 6)
        #expect(abs(v.length - 2) < 1e-12)
        #expect(abs(v.x(at: 6, width: 800) - 600) < 1e-9)  // 6 s was at 600 px of 800 before
        v.zoom(by: 1e9, around: 6)
        #expect(v.length == Viewport.minLength)
        v.zoom(by: 1e-9, around: 6)
        #expect(v.length == 8 && v.start == 0)
    }

    @Test func panClampsToTheClip() {
        var v = Viewport(total: 8)
        v.zoom(by: 8, around: 0)
        v.pan(by: -1)
        #expect(v.start == 0)
        v.pan(by: 100)
        #expect(v.end == 8)
    }

    @Test func showRangeAndTimeMapping() {
        var v = Viewport(total: 8)
        v.show(5.0...5.02, padding: 0.5)
        #expect(abs(v.length - 0.04) < 1e-12)
        #expect(abs(v.start - 4.99) < 1e-12)
        #expect(abs(v.time(atX: 500, width: 1000) - 5.01) < 1e-12)
        v.reset()
        #expect(v.start == 0 && v.length == 8)
    }
}

/// Files the test remover saw (tests run the remover synchronously on the main actor).
final class Removed: @unchecked Sendable {
    var names: [String] = []
}

@MainActor
struct ReviewModelTests {
    func library(_ f: Fixture) throws -> SnapshotLibrary {
        for (unix, reviewed) in [(100, false), (200, true), (300, false)] {
            try f.wav("snapshot-\(unix)-missed", seconds: 2, left: { i in i == 48_000 ? 0.9 : 0 })
            try f.json("snapshot-\(unix)-missed", ["label": "missed", "unix_time": unix, "seconds": 2.0, "reviewed": reviewed, "keep": "me"])
        }
        return SnapshotLibrary(folder: f.dir)
    }

    @Test func reloadOpensNewestAndNavigates() throws {
        let f = try Fixture()
        let m = ReviewModel(library: try library(f))
        m.reload()
        #expect(m.clips.count == 3)
        #expect(m.current?.stem == "snapshot-300-missed")
        #expect(m.audio?.duration == 2)
        #expect(m.annotation?.hint == 0...1.7)  // old-file default, clamped at 0
        m.selectNext()
        #expect(m.current?.stem == "snapshot-200-missed")
        m.selectNext()
        m.selectNext()  // stays on the last
        #expect(m.selectedIndex == 2)
        m.selectPrevious()
        #expect(m.selectedIndex == 1)
    }

    @Test func editsAreSavedToTheSidecar() throws {
        let f = try Fixture()
        let m = ReviewModel(library: try library(f))
        m.reload()
        let id = try req(m.addRegion(from: 1.0, to: 1.01))
        #expect(m.selectedRegion == id)
        m.setClass(.breath)
        var side = try f.read("snapshot-300-missed")
        #expect((side["regions"] as? [[String: Any]])?.first?["class"] as? String == "breath")
        #expect(side["keep"] as? String == "me")
        #expect(m.clips[0].regionCount == 1)

        m.deleteSelectedRegion()
        #expect(m.selectedRegion == nil)
        side = try f.read("snapshot-300-missed")
        #expect((side["regions"] as? [Any])?.isEmpty == true)
    }

    @Test func returnMarksReviewedAndAdvances() throws {
        let f = try Fixture()
        let m = ReviewModel(library: try library(f))
        m.reload()
        m.markReviewedAndAdvance()
        #expect(try f.read("snapshot-300-missed")["reviewed"] as? Bool == true)
        #expect(m.clips[0].reviewed)
        #expect(m.current?.stem == "snapshot-200-missed")
        m.setReviewed(false)
        #expect(try f.read("snapshot-200-missed")["reviewed"] as? Bool == false)
    }

    /// A / X act on the proposal last shown, else the first undecided one, and save at once.
    @Test func acceptAndRejectShortcutsVerifyAutoClips() throws {
        let f = try Fixture()
        try f.wav("snapshot-400-auto", seconds: 8)
        try f.json(
            "snapshot-400-auto",
            [
                "label": "auto", "unix_time": 400, "seconds": 8.0,
                "proposals": [
                    ["start_s": 2.0, "end_s": 2.02, "class": "lip_smack", "module": "desmack"],
                    ["start_s": 5.0, "end_s": 5.03, "class": "lip_smack", "module": "desmack"],
                ],
            ])
        let m = ReviewModel(library: SnapshotLibrary(folder: f.dir))
        m.reload()
        #expect(m.current?.labelTitle == "Auto")
        m.acceptNextProposal()  // the first undecided: 2.0 s
        var side = try f.read("snapshot-400-auto")
        #expect((side["regions"] as? [[String: Any]])?.first?["start_s"] as? Double == 2.0)
        m.rejectNextProposal()  // the next undecided: 5.0 s
        side = try f.read("snapshot-400-auto")
        #expect((side["rejected"] as? [[String: Any]])?.first?["start_s"] as? Double == 5.0)
        // Shown proposal wins: flip the first to a false positive.
        m.showProposal(0)
        m.rejectNextProposal()
        side = try f.read("snapshot-400-auto")
        #expect((side["regions"] as? [Any])?.isEmpty == true)
        #expect((side["rejected"] as? [Any])?.count == 2)
        #expect(m.clips[0].regionCount == 0)
        // Nothing left to decide: the keys do nothing.
        m.acceptNextProposal()
        #expect((try f.read("snapshot-400-auto")["regions"] as? [Any])?.isEmpty == true)
    }

    /// X on a proposal must not drop a different region the user has selected (UI review).
    @Test func rejectKeepsAnUnrelatedSelection() throws {
        let f = try Fixture()
        try f.wav("snapshot-401-auto", seconds: 8)
        try f.json(
            "snapshot-401-auto",
            ["label": "auto", "unix_time": 401, "seconds": 8.0, "proposals": [["start_s": 2.0, "end_s": 2.02, "class": "lip_smack"]]])
        let m = ReviewModel(library: SnapshotLibrary(folder: f.dir))
        m.reload()
        m.acceptNextProposal()
        let mine = try req(m.addRegion(from: 6.0, to: 6.1))
        m.reject(proposal: 0)
        #expect(m.selectedRegion == mine)
        #expect(m.annotation?.regions.map(\.id) == [mine])
    }

    /// An auto clip deleted underneath the window (hourly expiry) must not get its sidecar back.
    @Test func editingADeletedClipDoesNotRecreateItsSidecar() throws {
        let f = try Fixture()
        try f.wav("snapshot-402-auto", seconds: 8)
        try f.json(
            "snapshot-402-auto",
            ["label": "auto", "unix_time": 402, "seconds": 8.0, "proposals": [["start_s": 2.0, "end_s": 2.02, "class": "lip_smack"]]])
        let m = ReviewModel(library: SnapshotLibrary(folder: f.dir))
        m.reload()
        try FileManager.default.removeItem(at: f.dir.appendingPathComponent("snapshot-402-auto.wav"))
        try FileManager.default.removeItem(at: f.dir.appendingPathComponent("snapshot-402-auto.json"))
        m.acceptNextProposal()
        #expect(!FileManager.default.fileExists(atPath: f.dir.appendingPathComponent("snapshot-402-auto.json").path))
        #expect(m.errorMessage != nil)
    }

    @Test func zoomToRegion() throws {
        let f = try Fixture()
        let m = ReviewModel(library: try library(f))
        m.reload()
        let id = try req(m.addRegion(from: 1.0, to: 1.02))
        m.zoom(toRegion: id)
        #expect(m.viewport.start < 1.0 && m.viewport.end > 1.02 && m.viewport.length < 0.1)
    }

    /// Deleting a clip removes its wav + sidecar (to the Trash in the app; plain removal here),
    /// records it in deleted.txt so a later import doesn't bring it back, and opens the next one.
    @Test func deleteClipRemovesFilesRecordsAndOpensTheNext() throws {
        let f = try Fixture()
        let removed = Removed()
        let lib = try library(f).withRemover { url in
            removed.names.append(url.lastPathComponent)
            try FileManager.default.removeItem(at: url)
        }
        let m = ReviewModel(library: lib)
        m.reload()
        let size = try FileManager.default.attributesOfItem(atPath: f.dir.appendingPathComponent("snapshot-300-missed.wav").path)[.size] as? Int
        m.deleteCurrentClip()
        #expect(removed.names == ["snapshot-300-missed.wav", "snapshot-300-missed.json"])
        #expect(m.clips.map(\.stem) == ["snapshot-200-missed", "snapshot-100-missed"])
        #expect(m.current?.stem == "snapshot-200-missed")
        let record = try String(contentsOf: f.dir.appendingPathComponent(SnapshotLibrary.deletedFile), encoding: .utf8)
        #expect(record.contains("snapshot-300-missed\t\(try req(size))\n"))
        // Deleting the last one in the list opens the one above it; deleting all leaves nothing open.
        m.select(1)
        m.deleteCurrentClip()
        #expect(m.current?.stem == "snapshot-200-missed")
        m.deleteCurrentClip()
        #expect(m.clips.isEmpty && m.current == nil && m.audio == nil)
        m.deleteCurrentClip()  // nothing open: no-op
        #expect(try String(contentsOf: f.dir.appendingPathComponent(SnapshotLibrary.deletedFile), encoding: .utf8)
            .split(separator: "\n").filter { !$0.hasPrefix("#") }.count == 3)
    }

    @Test func deleteSpecificRowKeepsTheOpenClip() throws {
        let f = try Fixture()
        let lib = try library(f).withRemover { try FileManager.default.removeItem(at: $0) }
        let m = ReviewModel(library: lib)
        m.reload()
        m.deleteClip(at: 2)
        #expect(m.clips.map(\.stem) == ["snapshot-300-missed", "snapshot-200-missed"])
        #expect(m.current?.stem == "snapshot-300-missed")
    }

    @Test func unreadableClipShowsAnError() throws {
        let f = try Fixture()
        try "garbage".write(to: f.dir.appendingPathComponent("snapshot-5-missed.wav"), atomically: true, encoding: .utf8)
        let m = ReviewModel(library: SnapshotLibrary(folder: f.dir))
        m.reload()
        #expect(m.audio == nil)
        #expect(m.errorMessage?.contains("snapshot-5-missed") == true)
    }
}

/// "Reviewed" is a promise (ADR 0017): every mouth noise in the clip is marked, so all unmarked
/// audio becomes negative training data. The Review header says so next to the toggle.
struct ReviewedHintTests {
    @Test func captionAsksForEveryMouthNoiseUntilReviewed() {
        #expect(ReviewedHint.caption(reviewed: false) == "Mark every mouth noise first")
        #expect(ReviewedHint.caption(reviewed: true) == "Every mouth noise marked")
    }

    @Test func explanationSaysUnmarkedAudioIsTrainingDataAndNamesReturn() {
        let text = ReviewedHint.explanation
        #expect(text.contains("every mouth noise"))
        #expect(text.contains("unmarked"))
        #expect(text.contains("training"))
        #expect(text.contains("Return"))
    }
}
