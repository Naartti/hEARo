import Foundation
import Observation

/// View model for the snapshot review window: the clip list, the open clip's audio and
/// annotation, the selected region, zoom, and playback. Every edit is saved to the sidecar
/// when it is finished (drags save on release), so there is no Save button and nothing to lose.
@MainActor
@Observable
public final class ReviewModel {
    public let library: SnapshotLibrary
    public private(set) var clips: [SnapshotInfo] = []
    public private(set) var selectedIndex: Int?
    public private(set) var audio: AudioClip?
    public private(set) var annotation: Annotation?
    public var selectedRegion: UUID?
    public var viewport = Viewport(total: 8)
    /// Click position; plain playback starts here.
    public var playhead: Double = 0
    /// The list row under the mouse (Space plays that clip).
    public var hoveredRow: Int?
    public private(set) var errorMessage: String?
    @ObservationIgnored public let player: (any ClipPlayback)?
    /// Spectrogram images of recently opened clips (observable on its own).
    @ObservationIgnored public let spectrograms = SpectrogramCache()
    /// The processing inspector (trial settings, offline render, A/B); nil without an engine.
    @ObservationIgnored public let inspector: ReviewInspectorModel?
    /// Previews hold in-memory data; `reload()` must not replace it with the (empty) folder.
    @ObservationIgnored private var isPreview = false

    /// `live` + `renderer`: enable the processing inspector (the app passes its `AppModel`).
    public init(
        library: SnapshotLibrary, player: (any ClipPlayback)? = nil, live: (any LiveSettings)? = nil,
        renderer: ClipRenderer? = nil, renderDebounce: Duration = .milliseconds(150)
    ) {
        self.library = library
        self.player = player
        if let live, let renderer {
            inspector = ReviewInspectorModel(renderer: renderer, live: live, player: player, debounce: renderDebounce)
        } else {
            inspector = nil
        }
    }

    public var current: SnapshotInfo? { selectedIndex.map { clips[$0] } }

    /// Re-read the folder, keeping the open clip if it still exists.
    public func reload() {
        if isPreview { return }
        let keep = current?.stem
        clips = library.list()
        if let keep, let i = clips.firstIndex(where: { $0.stem == keep }) {
            selectedIndex = i
        } else if !clips.isEmpty {
            select(0)
        } else {
            selectedIndex = nil
            audio = nil
            annotation = nil
        }
        // After the window was closed and reopened the clip is the same but needs a new render.
        inspector?.open(audio != nil ? current?.wavURL : nil)
    }

    public func select(_ index: Int) {
        guard clips.indices.contains(index) else { return }
        if index == selectedIndex, audio != nil { return }
        player?.stop()
        selectedIndex = index
        selectedRegion = nil
        let info = clips[index]
        do {
            let clip = try SnapshotLibrary.loadAudio(info.wavURL)
            let side = try SnapshotLibrary.readSidecar(info.jsonURL)
            audio = clip
            annotation = Annotation(sidecar: side, duration: clip.duration, sampleRate: clip.sampleRate)
            viewport = Viewport(total: clip.duration)
            playhead = annotation?.hint.lowerBound ?? 0
            try player?.load(info.wavURL)
            errorMessage = nil
            inspector?.open(info.wavURL)
        } catch {
            audio = nil
            annotation = nil
            errorMessage = "Couldn't open \(info.stem): \(error.localizedDescription)"
            inspector?.open(nil)
        }
    }

    public func selectNext() { step(+1) }
    public func selectPrevious() { step(-1) }

    private func step(_ d: Int) {
        guard !clips.isEmpty else { return }
        let i = (selectedIndex ?? -d).advanced(by: d)
        select(min(max(0, i), clips.count - 1))
    }

    // MARK: Editing (each finished edit is saved)

    /// Change the annotation in memory; call `commit()` when the gesture ends.
    public func edit(_ change: (inout Annotation) -> Void) {
        guard var a = annotation else { return }
        change(&a)
        annotation = a
    }

    /// Persist the current annotation to the sidecar.
    public func commit() {
        guard let a = annotation, let i = selectedIndex else { return }
        do {
            try SnapshotLibrary.save(a, to: clips[i].jsonURL)
            clips[i].reviewed = a.reviewed
            clips[i].regionCount = a.regions.count
            errorMessage = nil
        } catch {
            errorMessage = "Couldn't save: \(error.localizedDescription)"
        }
    }

    @discardableResult
    public func addRegion(from a: Double, to b: Double) -> UUID? {
        var id: UUID?
        edit { id = $0.addRegion(from: a, to: b) }
        if let id { selectedRegion = id }
        commit()
        return id
    }

    public func setClass(_ cls: RegionClass) {
        guard let id = selectedRegion else { return }
        edit { $0.setClass(id, cls) }
        commit()
    }

    public func deleteSelectedRegion() {
        guard let id = selectedRegion else { return }
        edit { $0.delete(id) }
        selectedRegion = nil
        commit()
    }

    public func accept(proposal index: Int) {
        var id: UUID?
        edit { id = $0.accept(proposal: index) }
        selectedRegion = id
        commit()
    }

    /// Return: mark the clip done and move on to the next one.
    public func markReviewedAndAdvance() {
        setReviewed(true)
        selectNext()
    }

    public func setReviewed(_ on: Bool) {
        edit { $0.reviewed = on }
        commit()
    }

    // MARK: Deleting clips

    /// Delete the open clip (to the Trash) and open the next one (or the previous, at the end).
    public func deleteCurrentClip() {
        guard let i = selectedIndex else { return }
        deleteClip(at: i)
    }

    /// Delete the clip in list row `index` (to the Trash). Keeps the open clip unless it was this one.
    public func deleteClip(at index: Int) {
        guard clips.indices.contains(index) else { return }
        let wasOpen = index == selectedIndex
        if wasOpen { player?.stop() }
        do {
            try library.delete(clips[index])
        } catch {
            errorMessage = "Couldn't delete \(clips[index].stem): \(error.localizedDescription)"
            return
        }
        clips.remove(at: index)
        hoveredRow = nil
        guard let open = selectedIndex else { return }
        if clips.isEmpty {
            selectedIndex = nil
            audio = nil
            annotation = nil
            selectedRegion = nil
            inspector?.open(nil)
        } else if wasOpen {
            selectedIndex = nil
            select(min(index, clips.count - 1))
        } else if index < open {
            selectedIndex = open - 1
        }
    }

    public func zoom(toRegion id: UUID) {
        guard let r = annotation?.region(id) else { return }
        viewport.show(r.start...r.end)
    }

    // MARK: Playback

    /// Space: plays/pauses. Over a list row that isn't open, opens it and plays it. A selected
    /// region loops; otherwise the clip plays from the playhead to the end.
    public func togglePlay() {
        guard let player else { return }
        if let row = hoveredRow, row != selectedIndex {
            select(row)
        } else if player.isPlaying {
            if let t = player.currentTime, !player.looping { playhead = t }
            player.stop()
            return
        }
        do {
            if let id = selectedRegion, let r = annotation?.region(id) {
                try player.play(from: r.start, to: r.end, loop: true)
            } else {
                let duration = audio?.duration ?? 0
                if playhead >= duration - 0.01 { playhead = 0 }
                try player.play(from: playhead)
            }
        } catch {
            errorMessage = "Playback failed: \(error.localizedDescription)"
        }
    }
}

extension ReviewModel {
    /// The Review tab went away (another tab, or the window closed): stop playback but keep the
    /// open clip, render and spectrograms, so flipping tabs costs nothing.
    public func tabHidden() {
        player?.stop()
    }

    /// The hEARo window closed: drop caches and temporary renders (the app lives on in the
    /// menu bar).
    public func windowClosed() {
        player?.stop()
        spectrograms.removeAll()
        inspector?.windowClosed()
    }
}

extension ReviewModel {
    /// Populated with a synthetic clip for SwiftUI previews (no files). `live`: also show the
    /// processing inspector with a synthetic render.
    public static func preview(live: (any LiveSettings)? = nil) -> ReviewModel {
        let m = ReviewModel(
            library: SnapshotLibrary(folder: URL(fileURLWithPath: "/nonexistent")), live: live, renderer: FakeClipRenderer())
        let clip = AudioClip.synthetic()
        let url = URL(fileURLWithPath: "/nonexistent/snapshot-1763000000-missed.wav")
        m.inspector?.showPreview(.preview(for: clip), clip: url)
        m.clips = [
            SnapshotInfo(
                stem: "snapshot-1763000000-missed", wavURL: url, jsonURL: url, date: Date(timeIntervalSince1970: 1_763_000_000),
                label: "missed", reviewed: false, proposalCount: 1, regionCount: 1),
            SnapshotInfo(
                stem: "snapshot-1762990000-false_hit", wavURL: url, jsonURL: url,
                date: Date(timeIntervalSince1970: 1_762_990_000), label: "false_hit", reviewed: true, proposalCount: 0,
                regionCount: 0),
        ]
        m.isPreview = true
        m.selectedIndex = 0
        m.audio = clip
        var a = Annotation(duration: clip.duration, sampleRate: clip.sampleRate, pressAt: clip.duration)
        a.proposals = [
            Proposal(
                id: 0, start: 6.598, end: 6.612, cls: .lipSmack, classString: "lip_smack", module: "declick", dbRemoved: 9.5,
                inPressWindow: true)
        ]
        a.addRegion(from: 5.899, to: 5.913)
        m.annotation = a
        m.viewport = Viewport(total: clip.duration)
        m.selectedRegion = a.regions.first?.id
        return m
    }
}
