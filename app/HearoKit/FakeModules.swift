import Foundation

// The fake module catalog, settings rules and offline renderer behind `FakeEngine` (previews,
// tests and UI work without Rust). Mirrors the Rust catalog (`module_catalog()`); the real one
// always comes from Rust.

extension FakeEngine {
    /// A catalog shaped like the Rust one.
    public nonisolated static let catalog: [ModuleInfo] = [
        ModuleInfo(
            id: "dehum", name: "De-hum", description: "Removes DC offset and 50/60 Hz mains hum with its harmonics.",
            cleanup: true, alwaysOn: false,
            params: [
                ParamInfo(
                    id: "mains", name: "Mains", unit: "Hz", min: 0, max: 60, defaultValue: 0,
                    choices: [
                        ParamChoice(value: 0, label: "Auto"), ParamChoice(value: 50, label: "50 Hz"),
                        ParamChoice(value: 60, label: "60 Hz"),
                    ]),
                ParamInfo(id: "harmonics", name: "Harmonics", unit: "", min: 1, max: 8, defaultValue: 8, step: 1),
                ParamInfo(id: "threshold", name: "Threshold", unit: "dB", min: 6, max: 30, defaultValue: 12),
                ParamInfo(id: "amount", name: "Amount", unit: "%", min: 0, max: 1, defaultValue: 1, primary: true),
            ]),
        ModuleInfo(
            id: "deplosive", name: "De-plosive", description: "Softens low \"p\"/\"b\" pops and breath thumps.",
            cleanup: true, alwaysOn: false,
            params: [
                ParamInfo(id: "threshold", name: "Threshold", unit: "dB", min: 6, max: 30, defaultValue: 12),
                ParamInfo(id: "cutoff", name: "Cutoff", unit: "Hz", min: 80, max: 250, defaultValue: 150),
                ParamInfo(id: "max_cut", name: "Max cut", unit: "dB", min: 3, max: 36, defaultValue: 24, primary: true),
                ParamInfo(id: "release", name: "Release", unit: "ms", min: 10, max: 300, defaultValue: 60),
            ]),
        ModuleInfo(
            id: "declick", name: "De-click", description: "Repairs tiny mouth and tongue clicks inside speech.",
            cleanup: true, alwaysOn: false,
            params: [
                ParamInfo(
                    id: "sensitivity", name: "Sensitivity", unit: "%", min: 0, max: 1, defaultValue: 0.5, primary: true)
            ]),
        ModuleInfo(
            id: "desmack", name: "De-smack", description: "Turns down lip smacks in the pauses between words.",
            cleanup: true, alwaysOn: false,
            params: [
                ParamInfo(id: "threshold", name: "Threshold", unit: "dB", min: 6, max: 40, defaultValue: 14),
                ParamInfo(id: "pause", name: "Pause depth", unit: "dB", min: 0, max: 40, defaultValue: 15),
                ParamInfo(id: "range", name: "Range", unit: "dB", min: 0, max: 48, defaultValue: 24, primary: true),
                ParamInfo(id: "below_speech", name: "Below speech", unit: "dB", min: 0, max: 48, defaultValue: 22),
            ]),
        ModuleInfo(
            id: "deess", name: "De-ess", description: "Tames harsh \"s\" and \"sh\" sounds.",
            cleanup: true, alwaysOn: false,
            params: [
                ParamInfo(id: "threshold", name: "Threshold", unit: "dB", min: -60, max: 0, defaultValue: -40),
                ParamInfo(id: "range", name: "Range", unit: "dB", min: 0, max: 24, defaultValue: 12, primary: true),
                ParamInfo(id: "freq", name: "Frequency", unit: "Hz", min: 2000, max: 12000, defaultValue: 7000),
                ParamInfo(id: "q", name: "Q", unit: "", min: 0.5, max: 6, defaultValue: 1),
                ParamInfo(id: "attack", name: "Attack", unit: "ms", min: 0.1, max: 20, defaultValue: 1),
                ParamInfo(id: "release", name: "Release", unit: "ms", min: 10, max: 500, defaultValue: 60),
            ]),
        ModuleInfo(
            id: "limiter", name: "Limiter", description: "Always-on safety limiter that keeps peaks below the ceiling.",
            cleanup: false, alwaysOn: true,
            params: [
                ParamInfo(id: "ceiling", name: "Ceiling", unit: "dB", min: -6, max: 0, defaultValue: -1),
                ParamInfo(id: "release", name: "Release", unit: "ms", min: 20, max: 1000, defaultValue: 100),
            ]),
    ]

    /// Catalog defaults as settings.
    public nonisolated static let defaultSettings = EngineSettings(
        modules: catalog.map { m in
            ModuleSetting(
                key: m.id, enabled: true, params: m.params.map { ParamValue(key: $0.id, value: $0.defaultValue) })
        })

    nonisolated static func paramInfo(_ module: String, _ param: String) -> ParamInfo? {
        catalog.first(where: { $0.id == module })?.params.first(where: { $0.id == param })
    }

    /// Clamp + snap like Rust's `ParamDesc::snap` (NaN → default).
    public nonisolated static func snap(_ p: ParamInfo, _ v: Float) -> Float { p.snap(v) }

    /// `settings` over the defaults: unknown keys ignored, missing ones default, values snapped,
    /// always-on modules on (what the Rust engine does).
    public nonisolated static func normalize(_ settings: EngineSettings) -> EngineSettings {
        var s = defaultSettings
        for info in catalog {
            guard let m = settings.module(info.id) else { continue }
            s.setEnabled(info.id, m.enabled || info.alwaysOn)
            for p in m.params {
                if let pi = info.params.first(where: { $0.id == p.key }) { s.set(info.id, p.key, snap(pi, p.value)) }
            }
        }
        return s
    }
}

extension RenderResult {
    /// A plausible render of `AudioClip.synthetic()` for previews: a little hum removed
    /// throughout, the "s" at 2.2 s de-essed, and the two smacks (5.9 s, 6.6 s) ducked.
    public static func preview(for clip: AudioClip, columnsPerSecond cps: Float = 500) -> RenderResult {
        let n = Int((clip.duration * Double(cps)).rounded(.up))
        let per = max(1, Int(clip.sampleRate / Double(cps)))
        var input = [Float](repeating: 0, count: n)
        for c in 0..<n {
            let s0 = c * per
            let s1 = min(clip.mono.count, s0 + per)
            if s0 < s1 { input[c] = clip.mono[s0..<s1].reduce(0) { max($0, abs($1)) } }
        }
        var rows = Array(repeating: [Float](repeating: 0, count: n), count: cleanupModuleKeys.count)
        var events: [RenderEvent] = []
        func cut(_ module: Int, _ from: Double, _ to: Double, _ db: Float, _ kind: String) {
            let c0 = Int(from * Double(cps))
            let c1 = min(n, Int(to * Double(cps)))
            for c in c0..<max(c0, c1) { rows[module][c] = db }
            events.append(
                RenderEvent(module: cleanupModuleKeys[module], kind: kind, start: from, duration: to - from, reductionDb: db))
        }
        for c in 0..<n where input[c] > 0.02 { rows[0][c] = 1.5 }
        cut(4, 2.2, 2.45, 7, "sibilance")
        cut(3, 5.9, 5.915, 14, "smack")
        cut(3, 6.6, 6.615, 12, "smack")
        cut(2, 3.4, 3.402, 6, "click")
        let output = (0..<n).map { c in input[c] * pow(10, -rows.reduce(0) { $0 + $1[c] } / 20) }
        return RenderResult(
            output: URL(fileURLWithPath: "/dev/null"), sampleRate: Int(clip.sampleRate), channels: 1, frames: clip.mono.count,
            columnsPerSecond: cps, inputPeaks: input, outputPeaks: output,
            reductions: zip(cleanupModuleKeys, rows).map { ModuleReduction(key: $0, gainReductionDb: $1) }, events: events,
            renderMs: 38)
    }
}

/// Fake offline render: copies the clip and returns synthetic columns (previews/tests).
public struct FakeClipRenderer: ClipRenderer {
    public init() {}

    public func render(
        _ input: URL, to output: URL, settings: EngineSettings, bypass: Bool, columnsPerSecond: Float
    ) throws -> RenderResult {
        if input != output {
            try? FileManager.default.removeItem(at: output)
            try FileManager.default.copyItem(at: input, to: output)
        }
        let n = Int(8 * columnsPerSecond)
        var w = FakeEngine.syntheticWaveform(columns: n, start: 0)
        for (k, key) in cleanupModuleKeys.enumerated() where bypass || settings.module(key)?.enabled == false {
            w.reductions[k].gainReductionDb = Array(repeating: 0, count: n)
        }
        let events: [RenderEvent] = w.reductions.flatMap { row in
            row.gainReductionDb.enumerated().compactMap { i, db in
                db > 3
                    ? RenderEvent(
                        module: row.key, kind: row.key == "desmack" ? "smack" : "other",
                        start: Double(i) / Double(columnsPerSecond), duration: 0.01, reductionDb: db) : nil
            }
        }
        return RenderResult(
            output: output, sampleRate: 48_000, channels: 2, frames: 8 * 48_000, columnsPerSecond: columnsPerSecond,
            inputPeaks: w.peaks, outputPeaks: bypass ? w.peaks : w.outputPeaks, reductions: w.reductions,
            events: events)
    }
}
