import Foundation
import HearoFFI
import HearoKit

/// `Engine` backed by the Rust engine through UniFFI. Calls are short and non-real-time.
@MainActor
public final class RustEngine: HearoKit.Engine {
    private let core: HearoEngine

    /// `modules`: module keys in chain order; `nil` = every user-facing module the engine has.
    public init(modules: [String]? = nil) {
        let keys = modules ?? availableModules().filter { $0 != "gain" }
        core = HearoEngine(modules: keys)
    }

    public func snapshot() -> EngineSnapshot {
        let s = core.status()
        var snap = EngineSnapshot(
            running: s.running,
            idle: s.idle,
            bypassed: s.bypassed,
            permission: PermissionState(s.permission),
            processingApps: Int(s.processingApps),
            addedLatencyMs: s.addedLatencyMs,
            cpuPercent: s.cpuPercent,
            outputDevice: s.outputDevice,
            snapshotsSaved: Int(s.snapshotsSaved),
            modules: s.modules.map {
                ModuleStatus(id: $0.key, name: $0.name, enabled: $0.enabled, triggers: $0.triggers,
                             gainReductionDb: $0.gainReductionDb)
            })
        snap.problem = s.problem
        return snap
    }

    public func setRunning(_ on: Bool) throws {
        if on { try core.start() } else { core.stop() }
    }

    public func setBypass(_ on: Bool) { core.setBypass(on: on) }

    public func setModuleEnabled(_ id: String, _ on: Bool) { core.setModuleEnabled(key: id, on: on) }

    public func saveSnapshot(falseHit: Bool) throws -> String { try core.saveSnapshot(falseHit: falseHit) }

    public func exportSnapshots(to url: URL) throws -> Int {
        Int(try HearoFFI.exportSnapshots(path: url.path))
    }

    public func importSnapshots(from url: URL) throws -> SnapshotImport {
        let r = try HearoFFI.importSnapshots(path: url.path)
        return SnapshotImport(added: Int(r.added), skipped: Int(r.skipped), renamed: Int(r.renamed))
    }

    public var snapshotFolder: URL {
        let path = HearoFFI.snapshotFolder()
        try? FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
        return URL(fileURLWithPath: path, isDirectory: true)
    }

    public func waveform(since: UInt64, maxColumns: Int) -> HearoKit.WaveformChunk {
        let w = core.waveform(since: since, maxColumns: UInt32(maxColumns))
        return HearoKit.WaveformChunk(
            next: w.next, peaks: w.peaks, removed: w.removed, outputPeaks: w.outputPeaks,
            reductions: w.reductions.map(HearoKit.ModuleReduction.init))
    }

    // MARK: Modules and parameters

    public func moduleCatalog() -> [HearoKit.ModuleInfo] { HearoFFI.moduleCatalog().map(HearoKit.ModuleInfo.init) }

    public func setParam(_ module: String, _ param: String, _ value: Float) {
        core.setParam(moduleKey: module, paramKey: param, value: value)
    }

    public func settings() -> HearoKit.EngineSettings { HearoKit.EngineSettings(core.settings()) }

    public func applySettings(_ settings: HearoKit.EngineSettings) {
        core.applySettings(settings: HearoFFI.EngineSettings(settings))
    }

    public nonisolated var renderer: ClipRenderer { RustClipRenderer() }
}

/// Offline render through the Rust chain (`render_clip`). Stateless and thread-safe.
public struct RustClipRenderer: ClipRenderer {
    public init() {}

    public func render(
        _ input: URL, to output: URL, settings: HearoKit.EngineSettings, bypass: Bool, columnsPerSecond: Float
    ) throws -> RenderResult {
        let r = try HearoFFI.renderClip(
            wavPath: input.path, outWavPath: output.path, settings: HearoFFI.EngineSettings(settings), bypass: bypass,
            columnsPerSecond: columnsPerSecond)
        return RenderResult(
            output: URL(fileURLWithPath: r.outputPath), sampleRate: Int(r.sampleRate), channels: Int(r.channels),
            frames: Int(r.frames), columnsPerSecond: r.columnsPerSecond, inputPeaks: r.inputPeaks,
            outputPeaks: r.outputPeaks, reductions: r.reductions.map(HearoKit.ModuleReduction.init),
            events: r.events.map {
                HearoKit.RenderEvent(
                    module: $0.module, kind: $0.kind, start: $0.startS, duration: $0.durationS,
                    reductionDb: $0.reductionDb)
            },
            renderMs: r.renderMs)
    }
}

// MARK: FFI ↔ HearoKit conversions

extension HearoKit.ModuleReduction {
    init(_ r: HearoFFI.ModuleReduction) { self.init(key: r.key, gainReductionDb: r.gainReductionDb) }
}

extension HearoKit.ModuleInfo {
    init(_ m: HearoFFI.ModuleInfo) {
        self.init(
            id: m.key, name: m.name, description: m.description, cleanup: m.cleanup, alwaysOn: m.alwaysOn,
            params: m.params.map {
                HearoKit.ParamInfo(
                    id: $0.key, name: $0.name, unit: $0.unit, min: $0.min, max: $0.max, defaultValue: $0.defaultValue,
                    step: $0.step, choices: $0.choices.map { ParamChoice(value: $0.value, label: $0.label) },
                    primary: $0.primary)
            })
    }
}

extension HearoKit.EngineSettings {
    init(_ s: HearoFFI.EngineSettings) {
        self.init(
            modules: s.modules.map {
                HearoKit.ModuleSetting(
                    key: $0.key, enabled: $0.enabled,
                    params: $0.params.map { HearoKit.ParamValue(key: $0.key, value: $0.value) })
            })
    }
}

extension HearoFFI.EngineSettings {
    init(_ s: HearoKit.EngineSettings) {
        self.init(
            modules: s.modules.map {
                HearoFFI.ModuleSetting(
                    key: $0.key, enabled: $0.enabled,
                    params: $0.params.map { HearoFFI.ParamValue(key: $0.key, value: $0.value) })
            })
    }
}

extension PermissionState {
    init(_ p: Permission) {
        switch p {
        case .authorized: self = .granted
        case .denied: self = .denied
        case .notDetermined: self = .notDetermined
        case .unknown: self = .unknown
        }
    }
}

import ServiceManagement

/// Launch at login via `SMAppService.mainApp` (macOS 13+; registered for the installed app).
@MainActor
public final class SystemLoginItem: LoginItem {
    public init() {}
    public var isEnabled: Bool { SMAppService.mainApp.status == .enabled }
    public func setEnabled(_ on: Bool) throws {
        if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
    }
}
