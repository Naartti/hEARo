import Foundation
import Observation

/// Where the module settings JSON lives (`SettingsFile` in the app, `MemorySettingsFile` in tests).
@MainActor
public protocol SettingsPersistence: AnyObject {
    /// The stored settings, or nil when there are none (or they can't be read).
    func load() -> EngineSettings?
    func save(_ settings: EngineSettings) throws
}

/// `~/Library/Application Support/hEARo/settings.json`: the same JSON as Rust's
/// `settings_to_json` (ADR 0015). Written atomically. Loading is tolerant: unknown keys are
/// ignored and missing ones get defaults when the engine applies them; a file that isn't JSON
/// at all is moved aside to `settings.corrupt.json` (kept for inspection) and defaults are used.
@MainActor
public final class SettingsFile: SettingsPersistence {
    public let url: URL

    public init(url: URL = SettingsFile.defaultURL) { self.url = url }

    public nonisolated static var defaultURL: URL {
        let support =
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Library/Application Support")
        return support.appendingPathComponent("hEARo", isDirectory: true).appendingPathComponent("settings.json")
    }

    /// Where an unreadable file is moved.
    public var corruptURL: URL { url.deletingLastPathComponent().appendingPathComponent("settings.corrupt.json") }

    public func load() -> EngineSettings? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        if let s = try? JSONDecoder().decode(EngineSettings.self, from: data) { return s }
        let fm = FileManager.default
        try? fm.removeItem(at: corruptURL)
        try? fm.moveItem(at: url, to: corruptURL)
        return nil
    }

    public func save(_ settings: EngineSettings) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try enc.encode(settings).write(to: url, options: .atomic)
    }
}

/// In-memory settings store (tests, previews). Records every save.
@MainActor
public final class MemorySettingsFile: SettingsPersistence {
    public var stored: EngineSettings?
    public private(set) var saves: [EngineSettings] = []
    public init(_ stored: EngineSettings? = nil) { self.stored = stored }
    public func load() -> EngineSettings? { stored }
    public func save(_ settings: EngineSettings) throws {
        saves.append(settings)
        stored = settings
    }
}

extension EngineSettings {
    /// Every module on, every parameter at its catalog default.
    public static func defaults(_ catalog: [ModuleInfo]) -> EngineSettings {
        EngineSettings(
            modules: catalog.map { m in
                ModuleSetting(key: m.id, params: m.params.map { ParamValue(key: $0.id, value: $0.defaultValue) })
            })
    }

    /// The value of `param` in `module`, or the catalog default when it isn't set.
    public func value(_ module: ModuleInfo, _ param: ParamInfo) -> Float {
        self.module(module.id)?.value(param.id) ?? param.defaultValue
    }

    /// Whether `module` is on (missing = on, like the engine).
    public func isEnabled(_ module: String) -> Bool { self.module(module)?.enabled ?? true }

    /// Parameter values of `module` back to the catalog defaults (on/off unchanged).
    public mutating func resetValues(of module: ModuleInfo) {
        for p in module.params { set(module.id, p.id, p.defaultValue) }
    }
}

/// The live module settings: what the popover's amount sliders and the hEARo window's Settings tab
/// show and change. Every change goes to the engine at once (live within one audio buffer) and
/// is saved to the settings file shortly after the last change (debounced, so dragging a slider
/// writes once). The engine clamps and snaps; the model then shows the engine's values.
@MainActor
@Observable
public final class ModuleSettingsModel {
    /// Every user-facing module in chain order, with its parameters.
    public let catalog: [ModuleInfo]
    public private(set) var settings: EngineSettings
    /// Set when saving failed (shown in the settings window).
    public private(set) var saveError: String?
    /// The settings came from the file (false on first launch or without a file).
    public let loadedFromFile: Bool

    @ObservationIgnored private let engine: any Engine
    @ObservationIgnored private let store: (any SettingsPersistence)?
    @ObservationIgnored private let saveDelay: Duration
    /// The pending debounced save (tests await it).
    @ObservationIgnored private(set) var pendingSave: Task<Void, Never>?
    @ObservationIgnored private var dirty = false

    public init(engine: any Engine, store: (any SettingsPersistence)? = nil, saveDelay: Duration = .milliseconds(400)) {
        self.engine = engine
        self.store = store
        self.saveDelay = saveDelay
        catalog = engine.moduleCatalog()
        let stored = store?.load()
        loadedFromFile = stored != nil
        if let stored { engine.applySettings(stored) }
        settings = engine.settings()
    }

    public var isPersistent: Bool { store != nil }

    public func module(_ key: String) -> ModuleInfo? { catalog.first(where: { $0.id == key }) }

    public func value(_ module: ModuleInfo, _ param: ParamInfo) -> Float { settings.value(module, param) }

    /// Change one parameter (live) and save soon.
    public func set(_ module: String, _ param: String, _ value: Float) {
        engine.setParam(module, param, value)
        changed()
    }

    /// Re-read after the engine's on/off flags changed elsewhere (the popover toggles), and save.
    public func enabledChanged() { changed() }

    /// One module's values back to defaults (on/off unchanged).
    public func reset(_ key: String) {
        guard let m = module(key) else { return }
        for p in m.params { engine.setParam(m.id, p.id, p.defaultValue) }
        changed()
    }

    /// Every module's values back to defaults (on/off unchanged).
    public func resetAll() {
        var s = engine.settings()
        for m in catalog { s.resetValues(of: m) }
        apply(s)
    }

    /// Replace all settings (the review window's "Apply these to live").
    public func apply(_ new: EngineSettings) {
        engine.applySettings(new)
        changed()
    }

    /// Push the current settings into the engine (at start; harmless when already there).
    public func applyToEngine() {
        guard store != nil else { return }
        engine.applySettings(settings)
        settings = engine.settings()
    }

    /// Save now if a save is pending (quit).
    public func flush() {
        pendingSave?.cancel()
        pendingSave = nil
        if dirty { saveNow() }
    }

    private func changed() {
        settings = engine.settings()
        guard store != nil else { return }
        dirty = true
        pendingSave?.cancel()
        let delay = saveDelay
        pendingSave = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            self?.saveNow()
        }
    }

    private func saveNow() {
        guard let store else { return }
        do {
            try store.save(settings)
            dirty = false
            saveError = nil
        } catch {
            saveError = "Couldn't save settings: \(error.localizedDescription)"
        }
    }
}

/// How a parameter value reads in the UI ("24 dB", "50%", "7.5 kHz", "Auto").
public enum ParamFormat {
    public static func string(_ value: Float, _ p: ParamInfo, locale: Locale = .current) -> String {
        if let c = p.choices.min(by: { abs($0.value - value) < abs($1.value - value) }) { return c.label }
        let v = Double(value)
        func num(_ x: Double, _ digits: ClosedRange<Int>) -> String {
            x.formatted(.number.precision(.fractionLength(digits)).grouping(.never).locale(locale))
                .replacingOccurrences(of: "-", with: "\u{2212}")
        }
        switch p.unit {
        case "%":
            return (v.rounded(toPlaces: 2)).formatted(.percent.precision(.fractionLength(0)).locale(locale))
        case "dB": return "\(num(v, 0...1)) dB"
        case "Hz": return abs(v) >= 1000 ? "\(num(v / 1000, 0...1)) kHz" : "\(num(v, 0...0)) Hz"
        case "ms": return abs(v) < 10 ? "\(num(v, 0...1)) ms" : "\(num(v, 0...0)) ms"
        case "ratio": return "\(num(v, 0...1)):1"
        default: return p.step >= 1 ? num(v, 0...0) : num(v, 0...2)
        }
    }
}

extension Double {
    fileprivate func rounded(toPlaces n: Int) -> Double {
        let f = pow(10, Double(n))
        return (self * f).rounded() / f
    }
}
