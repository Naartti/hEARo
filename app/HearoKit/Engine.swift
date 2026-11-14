import Foundation

/// Whether macOS has granted "System Audio Recording".
public enum PermissionState: Sendable, Equatable {
    case unknown
    case notDetermined
    case granted
    case denied
}

/// One processing module as the UI sees it.
public struct ModuleStatus: Identifiable, Sendable, Equatable {
    public let id: String
    public let name: String
    public var enabled: Bool
    public var triggers: UInt64
    public var gainReductionDb: Float

    public init(id: String, name: String, enabled: Bool, triggers: UInt64 = 0, gainReductionDb: Float = 0) {
        self.id = id
        self.name = name
        self.enabled = enabled
        self.triggers = triggers
        self.gainReductionDb = gainReductionDb
    }
}

/// A point-in-time view of the engine, polled by the UI (never pushed from the audio thread).
public struct EngineSnapshot: Sendable, Equatable {
    public var running: Bool
    /// Running, but no processed app is playing (the tap is idle).
    public var idle: Bool
    public var bypassed: Bool
    public var permission: PermissionState
    public var processingApps: Int
    public var addedLatencyMs: Double
    public var cpuPercent: Double
    public var outputDevice: String
    public var snapshotsSaved: Int
    /// Clips auto-saved around de-smack catches this session (the icon flashes when it grows).
    public var autoSavedSession: Int = 0
    /// Auto-saved clips in the last 24 hours (what the daily limit counts).
    public var autoSavedLast24h: Int = 0
    /// Why the engine isn't processing although switched on (e.g. the new output device can't
    /// be used; hEARo retries). Empty when fine.
    public var problem: String = ""
    public var modules: [ModuleStatus]

    public init(
        running: Bool = false, idle: Bool = false, bypassed: Bool = false, permission: PermissionState = .unknown,
        processingApps: Int = 0, addedLatencyMs: Double = 0, cpuPercent: Double = 0, outputDevice: String = "",
        snapshotsSaved: Int = 0, modules: [ModuleStatus] = []
    ) {
        self.running = running
        self.idle = idle
        self.bypassed = bypassed
        self.permission = permission
        self.processingApps = processingApps
        self.addedLatencyMs = addedLatencyMs
        self.cpuPercent = cpuPercent
        self.outputDevice = outputDevice
        self.snapshotsSaved = snapshotsSaved
        self.modules = modules
    }

    /// Sum of all module triggers (drives the "just removed something" icon glow).
    public var totalTriggers: UInt64 { modules.reduce(0) { $0 + $1.triggers } }
}

/// The control plane the UI talks to. `RustEngine` (HearoLive) wraps the Rust engine via
/// UniFFI; `FakeEngine` drives previews, tests and UI work without audio.
@MainActor
public protocol Engine: AnyObject {
    func snapshot() -> EngineSnapshot
    /// Start or stop processing. Starting can fail (e.g. permission denied).
    func setRunning(_ on: Bool) throws
    func setBypass(_ on: Bool)
    func setModuleEnabled(_ id: String, _ on: Bool)
    /// Save the last 8 s of unprocessed audio. Returns the file path.
    func saveSnapshot(falseHit: Bool) throws -> String
    /// Live waveform columns with index ≥ `since` (at most `maxColumns`, most recent).
    func waveform(since: UInt64, maxColumns: Int) -> WaveformChunk
    /// Zip all snapshots into `url`; returns how many.
    func exportSnapshots(to url: URL) throws -> Int
    /// Merge a snapshot zip from another Mac (never overwrites).
    func importSnapshots(from url: URL) throws -> SnapshotImport
    /// The folder snapshots live in.
    var snapshotFolder: URL { get }

    // MARK: Modules and parameters (ADR 0014)

    /// Every user-facing module in chain order, with its parameters.
    func moduleCatalog() -> [ModuleInfo]
    /// Set one parameter (clamped/snapped by the engine; unknown keys ignored). Live within one
    /// audio buffer and remembered across device switches.
    func setParam(_ module: String, _ param: String, _ value: Float)
    /// Enabled flags + parameter values of every module (persist as JSON).
    func settings() -> EngineSettings
    /// Replace all module settings (missing → defaults, unknown ignored). Bypass unaffected.
    func applySettings(_ settings: EngineSettings)
    /// Offline renderer for the review window (use from a background task).
    var renderer: ClipRenderer { get }

    // MARK: Auto-saving de-smack catches (ADR 0018)

    /// Turn auto-saving on/off and set its limits (the engine clamps them).
    func setAutoCapture(_ settings: AutoCaptureSettings)
    /// Defaults and ranges for the settings UI.
    var autoCaptureLimits: AutoCaptureLimits { get }
}

/// Auto-save settings: a clip around every de-smack catch, for the dataset (ADR 0018).
public struct AutoCaptureSettings: Equatable, Sendable {
    public var enabled: Bool
    /// At most this many clips per rolling 24 hours.
    public var dailyCap: Int
    /// Auto-saved clips you never touched are deleted after this many days.
    public var retentionDays: Int

    public init(enabled: Bool, dailyCap: Int, retentionDays: Int) {
        self.enabled = enabled
        self.dailyCap = dailyCap
        self.retentionDays = retentionDays
    }

    /// Within `limits` (both numbers ≥ 1).
    public func clamped(to limits: AutoCaptureLimits) -> AutoCaptureSettings {
        AutoCaptureSettings(
            enabled: enabled, dailyCap: min(max(dailyCap, 1), limits.maxDailyCap),
            retentionDays: min(max(retentionDays, 1), limits.maxRetentionDays))
    }
}

/// Defaults and ranges of `AutoCaptureSettings` (from the engine).
public struct AutoCaptureLimits: Equatable, Sendable {
    public var defaultDailyCap: Int
    public var maxDailyCap: Int
    public var defaultRetentionDays: Int
    public var maxRetentionDays: Int

    public init(defaultDailyCap: Int, maxDailyCap: Int, defaultRetentionDays: Int, maxRetentionDays: Int) {
        self.defaultDailyCap = defaultDailyCap
        self.maxDailyCap = maxDailyCap
        self.defaultRetentionDays = defaultRetentionDays
        self.maxRetentionDays = maxRetentionDays
    }

    /// The engine's values (`hearo_engine::autocapture`); fakes use these.
    public static let standard = AutoCaptureLimits(
        defaultDailyCap: 250, maxDailyCap: 2000, defaultRetentionDays: 31, maxRetentionDays: 365)

    /// Off, at the default limits.
    public var defaults: AutoCaptureSettings {
        AutoCaptureSettings(enabled: false, dailyCap: defaultDailyCap, retentionDays: defaultRetentionDays)
    }
}

/// Result of importing snapshots.
public struct SnapshotImport: Sendable, Equatable {
    public var added: Int
    public var skipped: Int
    public var renamed: Int
    public init(added: Int, skipped: Int, renamed: Int) {
        self.added = added
        self.skipped = skipped
        self.renamed = renamed
    }
}

/// "Launch at login" (SMAppService in the app; a fake in tests).
@MainActor
public protocol LoginItem: AnyObject {
    var isEnabled: Bool { get }
    func setEnabled(_ on: Bool) throws
}
