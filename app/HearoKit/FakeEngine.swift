import Foundation

/// The modules in chain order, for previews and tests.
public let previewModules: [ModuleStatus] = [
    ModuleStatus(id: "dehum", name: "De-hum", enabled: true),
    ModuleStatus(id: "deplosive", name: "De-plosive", enabled: true),
    ModuleStatus(id: "declick", name: "De-click", enabled: true),
    ModuleStatus(id: "desmack", name: "De-smack", enabled: true),
    ModuleStatus(id: "deess", name: "De-ess", enabled: true),
    ModuleStatus(id: "limiter", name: "Limiter", enabled: true),
]

/// In-memory engine for previews, snapshot tests and UI development without audio.
@MainActor
public final class FakeEngine: Engine {
    public private(set) var state: EngineSnapshot
    /// Every control call, in order, so tests can assert the UI called the engine correctly.
    public private(set) var calls: [String] = []
    /// Make the next `setRunning(true)` fail.
    public var startError: Error?

    public init(_ state: EngineSnapshot = EngineSnapshot(modules: previewModules)) {
        self.state = state
    }

    public func snapshot() -> EngineSnapshot { state }

    public func setRunning(_ on: Bool) throws {
        calls.append("running=\(on)")
        if on, let e = startError { throw e }
        state.running = on
    }

    public func setBypass(_ on: Bool) {
        calls.append("bypass=\(on)")
        state.bypassed = on
    }

    public func setModuleEnabled(_ id: String, _ on: Bool) {
        calls.append("\(id)=\(on)")
        if let i = state.modules.firstIndex(where: { $0.id == id }) {
            state.modules[i].enabled = on
        }
    }

    public func saveSnapshot(falseHit: Bool) throws -> String {
        calls.append("snapshot falseHit=\(falseHit)")
        guard state.running else { throw FakeError.notRunning }
        state.snapshotsSaved += 1
        return "/tmp/snapshot-\(state.snapshotsSaved).wav"
    }

    /// Columns the next `waveform` call returns (tests). `nil` = synthetic animation (previews).
    public var nextWaveform: WaveformChunk?
    private var fakeCursor: UInt64 = 0

    public func waveform(since: UInt64, maxColumns: Int) -> WaveformChunk {
        if let w = nextWaveform {
            nextWaveform = nil
            return w
        }
        guard state.running else { return WaveformChunk(next: since) }
        let chunk = FakeEngine.syntheticWaveform(columns: 2, start: fakeCursor)
        fakeCursor = chunk.next
        return chunk
    }

    public var exported: [URL] = []
    public var importResult = SnapshotImport(added: 3, skipped: 1, renamed: 0)

    public func exportSnapshots(to url: URL) throws -> Int {
        exported.append(url)
        return state.snapshotsSaved
    }

    public func importSnapshots(from url: URL) throws -> SnapshotImport { importResult }

    public var snapshotFolder: URL { URL(fileURLWithPath: "/tmp/hearo-snapshots") }

    // MARK: Modules and parameters (catalog and renderer in FakeModules.swift)

    /// Current module parameter values (starts at the catalog defaults).
    public private(set) var currentSettings = FakeEngine.defaultSettings

    public func moduleCatalog() -> [ModuleInfo] { FakeEngine.catalog }

    public func setParam(_ module: String, _ param: String, _ value: Float) {
        calls.append("\(module).\(param)=\(value)")
        guard let info = FakeEngine.paramInfo(module, param) else { return }
        currentSettings.set(module, param, FakeEngine.snap(info, value))
    }

    public func settings() -> EngineSettings {
        var s = currentSettings
        for i in s.modules.indices {
            if let m = state.modules.first(where: { $0.id == s.modules[i].key }) { s.modules[i].enabled = m.enabled }
        }
        return s
    }

    public func applySettings(_ settings: EngineSettings) {
        calls.append("applySettings")
        currentSettings = FakeEngine.normalize(settings)
        for m in currentSettings.modules {
            if let i = state.modules.firstIndex(where: { $0.id == m.key }) { state.modules[i].enabled = m.enabled }
        }
    }

    public nonisolated var renderer: ClipRenderer { FakeClipRenderer() }

    /// Speech-like synthetic columns (previews): an occasional smack ducked by de-smack, a
    /// plosive, a click, sibilance on some syllables and a little steady hum reduction, with the
    /// processed peak lowered by the deepest cut.
    public nonisolated static func syntheticWaveform(columns: Int, start: UInt64) -> WaveformChunk {
        var peaks: [Float] = []
        var outPeaks: [Float] = []
        var removed: [Bool] = []
        var gr = Array(repeating: [Float](), count: cleanupModuleKeys.count)
        for i in start..<(start + UInt64(columns)) {
            let t = Double(i) / 60
            let syllable = max(0, sin(t * 2 * .pi * 3.1)) * (0.6 + 0.4 * sin(t * 0.7))
            let isSmack = i % 150 == 40
            let sibilant = i % 97 >= 20 && i % 97 < 26 && syllable > 0.3
            let row: [Float] = [
                0.75,  // de-hum: a little steady hum (below the 1 dB "removed" mark)
                i % 211 == 7 ? 9 : 0,  // de-plosive
                i % 173 == 90 ? 6 : 0,  // de-click
                isSmack ? 18 : 0,  // de-smack
                sibilant ? Float(4 + 4 * syllable) : 0,  // de-ess
            ]
            let peak: Float = isSmack ? 0.55 : Float(0.004 + 0.3 * syllable)
            peaks.append(peak)
            outPeaks.append(peak * pow(10, -(row.max() ?? 0) / 20))
            removed.append(row.contains { $0 > 1 })
            for k in row.indices { gr[k].append(row[k]) }
        }
        return WaveformChunk(
            next: start + UInt64(columns), peaks: peaks, removed: removed, outputPeaks: outPeaks,
            reductions: zip(cleanupModuleKeys, gr).map { ModuleReduction(key: $0, gainReductionDb: $1) })
    }

    /// Replace the state (tests and previews).
    public func set(_ state: EngineSnapshot) { self.state = state }

    public enum FakeError: Error, LocalizedError {
        case notRunning
        public var errorDescription: String? { "hEARo isn't running" }
    }
}

extension EngineSnapshot {
    /// A busy, healthy engine for previews.
    public static let previewRunning = EngineSnapshot(
        running: true, permission: .granted, processingApps: 3, addedLatencyMs: 18, cpuPercent: 1.4,
        outputDevice: "MacBook Pro Speakers",
        modules: previewModules.map {
            var m = $0
            m.triggers = UInt64($0.id.count * 7)
            m.gainReductionDb = $0.id == "deess" ? 4.5 : 0
            return m
        })

    /// Permission denied.
    public static let previewDenied = EngineSnapshot(permission: .denied, modules: previewModules)
}

/// In-memory login item for tests and previews.
@MainActor
public final class FakeLoginItem: LoginItem {
    public private(set) var isEnabled = false
    public init() {}
    public func setEnabled(_ on: Bool) throws { isEnabled = on }
}
