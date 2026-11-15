import Foundation
import Observation

/// What the menu bar icon shows (see docs/DESIGN.md, icon state table).
public enum MenuBarIconState: Equatable, Sendable {
    case off
    case on
    /// Something was just removed: accent-colour glow.
    case removing
    /// A snapshot was just saved: checkmark badge.
    case snapshotSaved
    /// A clip around a de-smack catch was auto-saved: checkmark badge in the de-smack colour.
    case autoSaved
    case problem

    public var symbol: String {
        switch self {
        case .off: "ear"
        case .on, .removing: "ear.fill"
        case .snapshotSaved, .autoSaved: "ear.badge.checkmark"
        case .problem: "ear.trianglebadge.exclamationmark"
        }
    }

    /// How the icon is coloured.
    public enum Tint: Equatable, Sendable {
        /// A template image (follows the menu bar).
        case template
        case accent
        /// A module's colour (`ModuleStyle`).
        case module(String)
    }

    /// Momentary states are coloured; the rest is a template image.
    public var tint: Tint {
        switch self {
        case .removing, .snapshotSaved: .accent
        case .autoSaved: .module("desmack")
        case .off, .on, .problem: .template
        }
    }

    public var isAccent: Bool { tint != .template }
}

/// Where the user's choices are remembered (UserDefaults in the app, in-memory in tests).
@MainActor
public protocol SettingsStore: AnyObject {
    func bool(_ key: String) -> Bool?
    func set(_ value: Bool, for key: String)
    func int(_ key: String) -> Int?
    func set(_ value: Int, for key: String)
}

/// `UserDefaults`-backed settings.
@MainActor
public final class DefaultsStore: SettingsStore {
    private let defaults: UserDefaults
    public init(_ defaults: UserDefaults = .standard) { self.defaults = defaults }
    public func bool(_ key: String) -> Bool? { defaults.object(forKey: key) as? Bool }
    public func set(_ value: Bool, for key: String) { defaults.set(value, forKey: key) }
    public func int(_ key: String) -> Int? { defaults.object(forKey: key) as? Int }
    public func set(_ value: Int, for key: String) { defaults.set(value, forKey: key) }
}

/// In-memory settings for tests and previews.
@MainActor
public final class MemoryStore: SettingsStore {
    public var values: [String: Bool] = [:]
    public var ints: [String: Int] = [:]
    public init() {}
    public func bool(_ key: String) -> Bool? { values[key] }
    public func set(_ value: Bool, for key: String) { values[key] = value }
    public func int(_ key: String) -> Int? { ints[key] }
    public func set(_ value: Int, for key: String) { ints[key] = value }
}

/// View model for the menu bar. Owns no audio state; it polls the engine.
@MainActor
@Observable
public final class AppModel {
    public private(set) var snapshot: EngineSnapshot
    public private(set) var errorMessage: String?
    /// A short confirmation shown in the popover (export/import results).
    public private(set) var infoMessage: String?
    @ObservationIgnored private let loginItem: (any LoginItem)?
    @ObservationIgnored private let settings: (any SettingsStore)?
    public private(set) var launchAtLogin = false
    public private(set) var iconState: MenuBarIconState = .off
    private let engine: any Engine
    /// Locale for number formatting (the user's by default; tests pin one).
    public let locale: Locale
    private var flashUntil: Date = .distantPast
    private var flashState: MenuBarIconState = .on
    private var lastTriggers: UInt64 = 0
    private var lastAutoSaved = 0
    /// Auto-saving de-smack catches (Settings tab; ADR 0018). Off by default.
    public private(set) var autoCapture: AutoCaptureSettings
    @ObservationIgnored private var timer: Timer?
    @ObservationIgnored private var waveTimer: Timer?
    @ObservationIgnored private var waveCursor: UInt64 = 0
    /// Live views currently on screen (the popover, the window's Controls tab) → Reduce Motion.
    @ObservationIgnored private var liveViewers: [LiveViewer: Bool] = [:]
    /// The hEARo window's selected tab (`MainWindowView`).
    public var mainTab: MainTab = .controls
    /// In the Dock while the hEARo window is open (`MainWindow.swift`); tests inject a fake.
    @ObservationIgnored var dock = DockPresence()
    /// The live waveform (≈ 3 s at 60 columns/s). Its own observable, so the 30 Hz poll redraws
    /// only `WaveformView`, never the rest of the controls.
    public let liveWaveform = LiveWaveform(capacity: 180)
    public var waveform: WaveformBuffer { liveWaveform.buffer }
    /// Module on/off and parameter values (amount sliders, Settings tab), saved to
    /// `settingsFile` when one is given.
    public let moduleSettings: ModuleSettingsModel

    /// `settingsFile`: where module settings are persisted (`SettingsFile()` in the app).
    public init(
        engine: any Engine, locale: Locale = .current, loginItem: (any LoginItem)? = nil,
        settings: (any SettingsStore)? = nil, settingsFile: (any SettingsPersistence)? = nil
    ) {
        self.engine = engine
        self.locale = locale
        self.loginItem = loginItem
        self.settings = settings
        // Restore remembered choices before anything starts: the settings file if there is one,
        // else the older per-module on/off flags (migrated into the file on the first save).
        moduleSettings = ModuleSettingsModel(engine: engine, store: settingsFile)
        if let settings, !moduleSettings.loadedFromFile {
            var restored = false
            for m in engine.snapshot().modules {
                if let on = settings.bool("module.\(m.id)"), on != m.enabled {
                    engine.setModuleEnabled(m.id, on)
                    restored = true
                }
            }
            if restored { moduleSettings.enabledChanged() }
        }
        self.launchAtLogin = loginItem?.isEnabled ?? false
        // Auto-save: always handed over at launch — the engine sweeps old auto clips only once
        // it has the user's retention.
        let limits = engine.autoCaptureLimits
        let d = limits.defaults
        let stored = AutoCaptureSettings(
            enabled: settings?.bool(Self.autoEnabledKey) ?? d.enabled,
            dailyCap: settings?.int(Self.autoCapKey) ?? d.dailyCap,
            retentionDays: settings?.int(Self.autoRetentionKey) ?? d.retentionDays
        ).clamped(to: limits)
        self.autoCapture = stored
        engine.setAutoCapture(stored)
        self.snapshot = engine.snapshot()
        self.lastTriggers = snapshot.totalTriggers
        self.lastAutoSaved = snapshot.autoSavedSession
        updateIcon(now: Date())
    }

    /// Pull the latest state and update the icon. `now` is injectable for tests.
    public func refresh(now: Date = Date()) {
        snapshot = engine.snapshot()
        let triggers = snapshot.totalTriggers
        if triggers > lastTriggers {
            flash(.removing, for: 0.5, now: now)
        }
        lastTriggers = triggers
        if snapshot.autoSavedSession > lastAutoSaved {
            flash(.autoSaved, for: 1.2, now: now)
        }
        lastAutoSaved = snapshot.autoSavedSession
        updateIcon(now: now)
    }

    /// Poll at `hz` while running (cheap: one FFI call). Call once at launch.
    public func startPolling(hz: Double = 5) {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 1 / hz, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    /// Pull new waveform columns (called at ~30 Hz while the popover is visible).
    public func pollWaveform() {
        let chunk = engine.waveform(since: waveCursor, maxColumns: liveWaveform.buffer.capacity)
        if chunk.next < waveCursor { liveWaveform.buffer.clear() }  // engine restarted
        waveCursor = chunk.next
        if !chunk.peaks.isEmpty { liveWaveform.buffer.append(chunk) }
    }

    /// Start/stop the waveform poll with the live views (no UI work while none is on screen):
    /// it runs while the popover *or* the window's Controls tab is visible. Under Reduce Motion
    /// (in any visible view) the waveform updates 10×/s in chunks instead of scrolling smoothly.
    public func setLiveViewVisible(_ visible: Bool, viewer: LiveViewer, reduceMotion: Bool = false) {
        liveViewers[viewer] = visible ? reduceMotion : nil
        let interval: TimeInterval? =
            liveViewers.isEmpty ? nil : (liveViewers.values.contains(true) ? 0.1 : 1.0 / 30)
        guard interval != waveformPollInterval else { return }  // keep the running timer
        waveTimer?.invalidate()
        waveTimer = nil
        waveformPollInterval = interval
        guard let interval else { return }
        pollWaveform()
        waveTimer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollWaveform() }
        }
    }

    /// Seconds between waveform polls, `nil` while no live view is visible.
    @ObservationIgnored private(set) var waveformPollInterval: TimeInterval?
    var isPollingWaveform: Bool { waveTimer != nil }

    /// Select a tab of the hEARo window (the caller opens the window).
    public func show(_ tab: MainTab) { mainTab = tab }

    public var isOn: Bool { snapshot.running }

    /// Whether processing should start at launch (remembered; default on).
    public var shouldStartAtLaunch: Bool { settings?.bool("running") ?? true }

    public func setOn(_ on: Bool) {
        settings?.set(on, for: "running")
        if on { moduleSettings.applyToEngine() }
        do {
            try engine.setRunning(on)
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
        }
        refresh()
    }

    public func setModule(_ id: String, enabled: Bool) {
        settings?.set(enabled, for: "module.\(id)")
        engine.setModuleEnabled(id, enabled)
        moduleSettings.enabledChanged()
        refresh()
    }

    /// Momentary A/B: hold to hear unprocessed audio.
    public func setBypass(_ on: Bool) {
        engine.setBypass(on)
        refresh()
    }

    /// The "Missed it" (§) / "False hit" hotkeys.
    @discardableResult
    public func saveSnapshot(falseHit: Bool = false, now: Date = Date()) -> Bool {
        do {
            _ = try engine.saveSnapshot(falseHit: falseHit)
            errorMessage = nil
            flash(.snapshotSaved, for: 1.2, now: now)
            refresh(now: now)
            return true
        } catch {
            errorMessage = "Snapshot failed: \(error.localizedDescription)"
            refresh(now: now)
            return false
        }
    }

    private func flash(_ state: MenuBarIconState, for seconds: TimeInterval, now: Date) {
        // A save confirmation wins over a removal glow.
        if flashUntil > now, flashState == .snapshotSaved || flashState == .autoSaved, state == .removing { return }
        flashState = state
        flashUntil = now.addingTimeInterval(seconds)
    }

    private func updateIcon(now: Date) {
        if snapshot.permission == .denied || errorMessage != nil && !snapshot.running {
            iconState = .problem
        } else if !snapshot.running {
            iconState = .off
        } else if flashUntil > now {
            iconState = flashState
        } else {
            iconState = .on
        }
    }

    public var snapshotFolder: URL { engine.snapshotFolder }

    static let autoEnabledKey = "autoCapture.enabled"
    static let autoCapKey = "autoCapture.dailyCap"
    static let autoRetentionKey = "autoCapture.retentionDays"

    /// Limits for the settings UI.
    public var autoCaptureLimits: AutoCaptureLimits { engine.autoCaptureLimits }

    /// Change the auto-save settings (clamped), remember them and hand them to the engine.
    public func setAutoCapture(_ new: AutoCaptureSettings) {
        let s = new.clamped(to: engine.autoCaptureLimits)
        autoCapture = s
        settings?.set(s.enabled, for: Self.autoEnabledKey)
        settings?.set(s.dailyCap, for: Self.autoCapKey)
        settings?.set(s.retentionDays, for: Self.autoRetentionKey)
        engine.setAutoCapture(s)
    }

    /// Set one module parameter (live) and remember it.
    public func setParam(_ module: String, _ param: String, _ value: Float) {
        moduleSettings.set(module, param, value)
    }

    /// Replace every module setting (e.g. the review window's "Apply these to live").
    public func applySettings(_ new: EngineSettings) {
        moduleSettings.apply(new)
        for m in moduleSettings.catalog where !m.alwaysOn {
            settings?.set(moduleSettings.settings.isEnabled(m.id), for: "module.\(m.id)")
        }
        refresh()
    }

    /// Module display names by key ("desmack" → "De-smack").
    public var moduleNames: [String: String] {
        Dictionary(moduleSettings.catalog.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
    }

    /// Offline renderer for the review window.
    public var renderer: ClipRenderer { engine.renderer }

    public func exportSnapshots(to url: URL) {
        do {
            let n = try engine.exportSnapshots(to: url)
            infoMessage = "Exported \(n) snapshot\(n == 1 ? "" : "s") to \(url.lastPathComponent)"
            errorMessage = nil
        } catch {
            errorMessage = "Export failed: \(error.localizedDescription)"
        }
    }

    public func importSnapshots(from url: URL) {
        do {
            let r = try engine.importSnapshots(from: url)
            var parts = ["\(r.added) added"]
            if r.skipped > 0 { parts.append("\(r.skipped) already here") }
            if r.renamed > 0 { parts.append("\(r.renamed) renamed") }
            infoMessage = "Imported snapshots: " + parts.joined(separator: ", ")
            errorMessage = nil
        } catch {
            errorMessage = "Import failed: \(error.localizedDescription)"
        }
    }

    public var canLaunchAtLogin: Bool { loginItem != nil }

    public func setLaunchAtLogin(_ on: Bool) {
        guard let loginItem else { return }
        do {
            try loginItem.setEnabled(on)
            errorMessage = nil
        } catch {
            errorMessage = "Launch at login: \(error.localizedDescription)"
        }
        launchAtLogin = loginItem.isEnabled
    }

    /// Header status line, e.g. "Processing 3 apps · +18 ms · 1.4% CPU".
    public var statusLine: String {
        if snapshot.permission == .denied { return "Needs System Audio Recording permission" }
        if !snapshot.running { return errorMessage == nil ? "Off" : "Not running" }
        let latency = "+\(Int(snapshot.addedLatencyMs.rounded())) ms"
        let bypass = snapshot.bypassed ? " · bypassed" : ""
        if snapshot.idle || snapshot.processingApps == 0 {
            return "Waiting for audio · \(latency)\(bypass)"
        }
        let apps = snapshot.processingApps == 1 ? "1 app" : "\(snapshot.processingApps) apps"
        let cpu = snapshot.cpuPercent.formatted(.number.precision(.fractionLength(1)).locale(locale))
        return "Processing \(apps) · \(latency) · \(cpu)% CPU\(bypass)"
    }

    /// Under the status line: where the sound goes (hEARo follows the system output), or why
    /// it isn't processing right now.
    public var outputLine: String? {
        if !snapshot.problem.isEmpty { return "Paused, playing unprocessed: \(snapshot.problem)" }
        if snapshot.running, !snapshot.outputDevice.isEmpty { return "Output: \(snapshot.outputDevice)" }
        return nil
    }

    /// Back-compat for the label: the SF Symbol for the current icon state.
    public var menuBarSymbol: String { iconState.symbol }
}
