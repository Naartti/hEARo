import Foundation
import Observation

/// The live settings, as the review inspector sees them (`AppModel`; a fake in tests).
@MainActor
public protocol LiveSettings: AnyObject {
    var moduleCatalog: [ModuleInfo] { get }
    var liveSettings: EngineSettings { get }
    /// Make `settings` the live settings (applied to the engine and remembered).
    func applySettings(_ settings: EngineSettings)
}

extension AppModel: LiveSettings {
    public var moduleCatalog: [ModuleInfo] { moduleSettings.catalog }
    public var liveSettings: EngineSettings { moduleSettings.settings }
}

/// The review window's processing inspector: a trial copy of the module settings (starting from
/// the live ones) that the open clip is rendered with offline, and the A/B bypass that switches
/// playback between the original and that render at the same position. Edits re-render off the
/// main thread, debounced; a newer edit cancels the stale render (its result is dropped).
@MainActor
@Observable
public final class ReviewInspectorModel {
    public enum RenderState: Equatable, Sendable {
        case idle
        case rendering
        /// Done: how long the render took.
        case ready(ms: Double)
        case failed(String)
    }

    public let catalog: [ModuleInfo]
    /// The trial settings the clip is rendered with.
    public private(set) var settings: EngineSettings
    /// A/B: true = hear (and see) the original.
    public private(set) var bypass = false
    public private(set) var state: RenderState = .idle
    /// The latest render of the open clip (nil while none is ready).
    public private(set) var result: RenderResult?
    /// The open clip.
    public private(set) var clip: URL?
    /// The user changed the trial settings since they were last taken from live.
    public private(set) var edited = false

    @ObservationIgnored private let renderer: ClipRenderer
    @ObservationIgnored private let live: (any LiveSettings)?
    @ObservationIgnored private weak var player: (any ClipPlayback)?
    @ObservationIgnored private let debounce: Duration
    @ObservationIgnored private let folder: URL
    @ObservationIgnored private var generation = 0
    /// The pending render (tests await it).
    @ObservationIgnored private(set) var renderTask: Task<Void, Never>?

    /// Render columns per second (2 ms: smacks are 5–30 ms long).
    public static let columnsPerSecond: Float = 500

    public init(
        renderer: ClipRenderer, live: any LiveSettings, player: (any ClipPlayback)?, debounce: Duration = .milliseconds(150),
        folder: URL = FileManager.default.temporaryDirectory.appendingPathComponent("hEARo-review", isDirectory: true)
    ) {
        self.renderer = renderer
        self.live = live
        self.player = player
        self.debounce = debounce
        self.folder = folder
        catalog = live.moduleCatalog
        settings = live.liveSettings
    }

    /// The live settings the trial copy differs from (nil without a live source).
    public var liveSettings: EngineSettings? { live?.liveSettings }
    public var matchesLive: Bool { live.map { $0.liveSettings == settings } ?? true }

    /// Bands and events to draw: the render, unless bypassed (the original has none).
    public var overlay: RenderResult? { bypass ? nil : result }

    // MARK: Clip

    /// A clip was opened (nil = none): render it with the trial settings.
    public func open(_ url: URL?) {
        guard url != clip else { return }
        clip = url
        result = nil
        cancelRender()
        removeRenders(keeping: nil)
        if url == nil {
            state = .idle
        } else {
            scheduleRender()
        }
    }

    /// The window appeared: start from the live settings unless the user is mid-experiment.
    public func windowAppeared() {
        if !edited, let live, live.liveSettings != settings {
            settings = live.liveSettings
            scheduleRender()
        }
    }

    /// The window closed: stop rendering and delete the temporary renders.
    public func windowClosed() {
        cancelRender()
        clip = nil
        result = nil
        state = .idle
        removeRenders(keeping: nil)
    }

    // MARK: Editing

    public func set(_ module: String, _ param: String, _ value: Float) {
        guard let p = catalog.first(where: { $0.id == module })?.params.first(where: { $0.id == param }) else { return }
        let v = p.snap(value)
        guard settings.module(module)?.value(param) != v else { return }
        settings.set(module, param, v)
        changed()
    }

    public func setEnabled(_ module: String, _ on: Bool) {
        guard settings.isEnabled(module) != on else { return }
        settings.setEnabled(module, on)
        changed()
    }

    public func reset(_ module: String) {
        guard let m = catalog.first(where: { $0.id == module }) else { return }
        settings.resetValues(of: m)
        changed()
    }

    /// "Use my live settings".
    public func useLiveSettings() {
        guard let live else { return }
        settings = live.liveSettings
        edited = false
        scheduleRender()
    }

    /// "Apply these to live".
    public func applyToLive() {
        guard let live else { return }
        live.applySettings(settings)
        settings = live.liveSettings  // as the engine clamped them
        edited = false
    }

    // MARK: A/B

    public func setBypass(_ on: Bool) {
        bypass = on
        player?.setPlaysProcessed(!on)
    }

    public func toggleBypass() { setBypass(!bypass) }

    // MARK: Rendering

    /// Show a ready-made result (previews; no file, no player).
    func showPreview(_ r: RenderResult, clip url: URL) {
        clip = url
        result = r
        state = .ready(ms: r.renderMs)
        isPreview = true
    }

    @ObservationIgnored private var isPreview = false

    private func changed() {
        edited = !matchesLive  // back at the live values = not an experiment any more
        if !isPreview { scheduleRender() }  // previews have no file to render
    }

    // Pipeline: a request bumps `generation` and (re)starts a debounce; when it fires, a render
    // starts unless one is already running (the FFI render can't be interrupted), in which case
    // the newest request runs as soon as it finishes. A result is applied only if no newer
    // request came in meanwhile, so browsing clips quickly or dragging a slider never piles up
    // renders: at most one runs, plus one queued.
    @ObservationIgnored private var debounceTask: Task<Void, Never>?
    @ObservationIgnored private var inFlight: Task<Void, Never>?
    @ObservationIgnored private var queued = false

    private func cancelRender() {
        generation += 1
        debounceTask?.cancel()
        debounceTask = nil
        queued = false
        renderTask = inFlight
    }

    private func scheduleRender() {
        cancelRender()
        guard clip != nil else { return }
        state = .rendering
        let delay = debounce
        debounceTask = Task { [weak self] in
            if delay > .zero {
                try? await Task.sleep(for: delay)
                if Task.isCancelled { return }
            }
            self?.startRender()
        }
        renderTask = debounceTask
    }

    private func startRender() {
        debounceTask = nil
        if inFlight != nil {
            queued = true
            renderTask = inFlight
            return
        }
        guard let input = clip else { return }
        let token = generation
        let settings = settings
        let renderer = renderer
        let cps = Self.columnsPerSecond
        let output = folder.appendingPathComponent("render-\(UUID().uuidString).wav")
        let folder = folder
        let task = Task { [weak self] in
            // Render and decode off the main thread; only the buffer copy happens on it.
            let outcome = await Task.detached(priority: .userInitiated) { () -> Result<(RenderResult, DecodedAudio), Error> in
                Result {
                    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                    let r = try renderer.render(input, to: output, settings: settings, bypass: false, columnsPerSecond: cps)
                    return (r, try DecodedAudio(contentsOf: r.output))
                }
            }.value
            guard let self else { return }
            let finished = self.inFlight
            self.inFlight = nil
            if token == self.generation {
                self.finish(outcome)
            } else {
                try? FileManager.default.removeItem(at: output)  // stale
            }
            if self.queued {
                self.queued = false
                self.startRender()
            } else if self.renderTask == finished {
                self.renderTask = nil
            }
        }
        inFlight = task
        renderTask = task
    }

    /// Wait until no render is pending, running or queued (tests).
    func waitUntilIdle() async {
        while let t = renderTask {
            await t.value
            if renderTask == t || renderTask == nil { return }
        }
    }

    private func finish(_ outcome: Result<(RenderResult, DecodedAudio), Error>) {
        switch outcome {
        case .success(let (r, audio)):
            result = r
            state = .ready(ms: r.renderMs)
            removeRenders(keeping: r.output)
            do {
                try player?.loadProcessed(audio)
                player?.setPlaysProcessed(!bypass)
            } catch {
                state = .failed("Couldn't play the render: \(error.localizedDescription)")
            }
        case .failure(let e):
            result = nil
            state = .failed(e.localizedDescription)
            try? player?.loadProcessed(nil)
        }
    }

    private func removeRenders(keeping keep: URL?) {
        let fm = FileManager.default
        guard let files = try? fm.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil) else { return }
        for f in files where f.lastPathComponent.hasPrefix("render-") && f.standardizedFileURL != keep?.standardizedFileURL {
            try? fm.removeItem(at: f)
        }
    }
}

extension RenderResult {
    /// Outside-in bands for the stretch `start…end` (seconds): the loudest input peak, the output
    /// peak of that column, and each module's deepest cut over the columns it covers.
    public func bands(from start: Double, to end: Double) -> ReductionBands {
        let cps = Double(columnsPerSecond)
        let n = inputPeaks.count
        guard n > 0, cps > 0, end > start else { return .none }
        let c0 = max(0, Int((start * cps).rounded(.down)))
        let c1 = min(n - 1, max(c0, Int((end * cps).rounded(.up)) - 1))
        guard c0 < n else { return .none }
        var peak: Float = 0
        var out: Float = 0
        // Rows by key, so a band's index is always its `cleanupModuleKeys` index (its colour).
        let rows = reductions.compactMap { row in cleanupModuleKeys.firstIndex(of: row.key).map { ($0, row.gainReductionDb) } }
        var r = [Float](repeating: 0, count: cleanupModuleKeys.count)
        for c in c0...c1 {
            // The output of the column the stroke's peak comes from (not the loudest output
            // elsewhere in the stretch, which would hide a cut).
            if inputPeaks[c] > peak || c == c0 {
                peak = max(peak, inputPeaks[c])
                out = c < outputPeaks.count ? outputPeaks[c] : peak
            }
            for (k, db) in rows where c < db.count { r[k] = max(r[k], db[c]) }
        }
        return ReductionBands.compute(peak: peak, outPeak: outputPeaks.isEmpty ? nil : out, reductionsDb: r)
    }
}
