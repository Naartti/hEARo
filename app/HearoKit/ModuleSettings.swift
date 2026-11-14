import Foundation

/// Cleanup modules in chain order: the rows of `WaveformChunk.reductions` and
/// `RenderResult.reductions`. Colour them outside-in in this order.
public let cleanupModuleKeys = ["dehum", "deplosive", "declick", "desmack", "deess"]

/// One labelled value of a discrete parameter ("Auto", "50 Hz").
public struct ParamChoice: Sendable, Equatable, Hashable {
    public var value: Float
    public var label: String
    public init(value: Float, label: String) {
        self.value = value
        self.label = label
    }
}

/// One module parameter as a settings UI shows it (from the Rust module catalog).
public struct ParamInfo: Sendable, Equatable, Identifiable {
    /// Stable key (`range`, …).
    public var id: String
    public var name: String
    /// `"dB"`, `"Hz"`, `"ms"`, `"ratio"`, `"%"` (value 0…1; show ×100) or `""`.
    public var unit: String
    public var min: Float
    public var max: Float
    public var defaultValue: Float
    /// Slider step (`min + k·step`); 0 = continuous.
    public var step: Float
    /// Non-empty: a discrete parameter (use a picker).
    public var choices: [ParamChoice]
    /// The module's one "how much is removed" control (simple UI). Others are advanced.
    public var primary: Bool

    public init(
        id: String, name: String, unit: String, min: Float, max: Float, defaultValue: Float, step: Float = 0,
        choices: [ParamChoice] = [], primary: Bool = false
    ) {
        self.id = id
        self.name = name
        self.unit = unit
        self.min = min
        self.max = max
        self.defaultValue = defaultValue
        self.step = step
        self.choices = choices
        self.primary = primary
    }
}

extension ParamInfo {
    /// Clamp + snap to a legal value like Rust's `ParamDesc::snap` (NaN → default).
    public func snap(_ v: Float) -> Float {
        let c = v.isNaN ? defaultValue : Swift.min(max, Swift.max(min, v))
        if let best = choices.min(by: { abs($0.value - c) < abs($1.value - c) }) { return best.value }
        if step > 0 { return Swift.min(max, min + ((c - min) / step).rounded() * step) }
        return c
    }
}

/// One module as a settings UI shows it.
public struct ModuleInfo: Sendable, Equatable, Identifiable {
    /// Stable key (`desmack`, …).
    public var id: String
    public var name: String
    public var description: String
    /// Has a gain-reduction row (false for the limiter).
    public var cleanup: Bool
    /// No on/off toggle (the limiter).
    public var alwaysOn: Bool
    public var params: [ParamInfo]

    public init(
        id: String, name: String, description: String, cleanup: Bool, alwaysOn: Bool, params: [ParamInfo]
    ) {
        self.id = id
        self.name = name
        self.description = description
        self.cleanup = cleanup
        self.alwaysOn = alwaysOn
        self.params = params
    }

    /// The simple UI's control, if any.
    public var primaryParam: ParamInfo? { params.first(where: \.primary) }
}

/// Decodes an array element, or nil if it is malformed (one bad value in the settings file
/// drops that value, not the whole file).
private struct Lossy<T: Decodable>: Decodable {
    let value: T?
    init(from decoder: Decoder) throws { value = try? T(from: decoder) }
}

/// A parameter value in `EngineSettings`.
public struct ParamValue: Sendable, Equatable, Codable {
    public var key: String
    public var value: Float
    public init(key: String, value: Float) {
        self.key = key
        self.value = value
    }
}

/// One module's settings.
public struct ModuleSetting: Sendable, Equatable, Codable {
    public var key: String
    public var enabled: Bool
    public var params: [ParamValue]

    public init(key: String, enabled: Bool = true, params: [ParamValue] = []) {
        self.key = key
        self.enabled = enabled
        self.params = params
    }

    /// Tolerant like the Rust side: missing `enabled` = on, missing `params` = defaults.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        key = try c.decode(String.self, forKey: .key)
        enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        params = (try c.decodeIfPresent([Lossy<ParamValue>].self, forKey: .params))?.compactMap(\.value) ?? []
    }

    public func value(_ param: String) -> Float? { params.first(where: { $0.key == param })?.value }
}

/// Enabled flags + parameter values for every module. Same JSON as Rust's `settings_to_json`
/// (`{"modules":[{"key","enabled","params":[{"key","value"}]}]}`); persist it with
/// `JSONEncoder` / `Engine.decodeSettings`. Applying it is forgiving: unknown keys are ignored,
/// missing ones get defaults, values are clamped (ADR 0014). Bypass is not part of it.
public struct EngineSettings: Sendable, Equatable, Codable {
    public var modules: [ModuleSetting]

    public init(modules: [ModuleSetting] = []) { self.modules = modules }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        modules = (try c.decodeIfPresent([Lossy<ModuleSetting>].self, forKey: .modules))?.compactMap(\.value) ?? []
    }

    public func module(_ key: String) -> ModuleSetting? { modules.first(where: { $0.key == key }) }

    /// Set (or add) one parameter value.
    public mutating func set(_ module: String, _ param: String, _ value: Float) {
        if let i = modules.firstIndex(where: { $0.key == module }) {
            if let j = modules[i].params.firstIndex(where: { $0.key == param }) {
                modules[i].params[j].value = value
            } else {
                modules[i].params.append(ParamValue(key: param, value: value))
            }
        } else {
            modules.append(ModuleSetting(key: module, params: [ParamValue(key: param, value: value)]))
        }
    }

    /// Enable/disable (or add) one module.
    public mutating func setEnabled(_ module: String, _ on: Bool) {
        if let i = modules.firstIndex(where: { $0.key == module }) {
            modules[i].enabled = on
        } else {
            modules.append(ModuleSetting(key: module, enabled: on))
        }
    }
}

/// Per-column gain reduction of one cleanup module.
public struct ModuleReduction: Sendable, Equatable {
    /// Module key (see `cleanupModuleKeys`).
    public var key: String
    /// dB ≥ 0 per column (live: 0.25 dB steps).
    public var gainReductionDb: [Float]
    public init(key: String, gainReductionDb: [Float]) {
        self.key = key
        self.gainReductionDb = gainReductionDb
    }
}

/// Something a module removed in a rendered clip.
public struct RenderEvent: Sendable, Equatable {
    public var module: String
    /// `click`, `smack`, `plosive`, `sibilance`, `hum` or `limit`.
    public var kind: String
    public var start: TimeInterval
    public var duration: TimeInterval
    public var reductionDb: Float
    public init(module: String, kind: String, start: TimeInterval, duration: TimeInterval, reductionDb: Float) {
        self.module = module
        self.kind = kind
        self.start = start
        self.duration = duration
        self.reductionDb = reductionDb
    }
}

/// Result of rendering a clip offline (review window).
public struct RenderResult: Sendable, Equatable {
    /// The processed WAV (same rate, channels and length as the input; lines up sample for sample).
    public var output: URL
    public var sampleRate: Int
    public var channels: Int
    public var frames: Int
    public var columnsPerSecond: Float
    /// Original peak per column.
    public var inputPeaks: [Float]
    /// Processed peak per column.
    public var outputPeaks: [Float]
    /// One row per cleanup module, chain order (`cleanupModuleKeys`).
    public var reductions: [ModuleReduction]
    public var events: [RenderEvent]
    public var renderMs: Double

    public init(
        output: URL, sampleRate: Int, channels: Int, frames: Int, columnsPerSecond: Float, inputPeaks: [Float],
        outputPeaks: [Float], reductions: [ModuleReduction], events: [RenderEvent], renderMs: Double = 0
    ) {
        self.output = output
        self.sampleRate = sampleRate
        self.channels = channels
        self.frames = frames
        self.columnsPerSecond = columnsPerSecond
        self.inputPeaks = inputPeaks
        self.outputPeaks = outputPeaks
        self.reductions = reductions
        self.events = events
        self.renderMs = renderMs
    }
}

/// Renders clips offline. Not tied to the main actor: call it from a background task.
public protocol ClipRenderer: Sendable {
    /// Process the WAV at `input` with `settings` through every user-facing module and write the
    /// result to `output`. `bypass` = bit-exact copy path. Columns at `columnsPerSecond`.
    func render(
        _ input: URL, to output: URL, settings: EngineSettings, bypass: Bool, columnsPerSecond: Float
    ) throws -> RenderResult
}
