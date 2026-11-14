import Foundation
import Testing

@testable import HearoKit

/// The settings file, debounced saving, restore/apply at launch, and value formatting.
@MainActor
struct SettingsStoreTests {
    private func tempFile() -> SettingsFile {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("hearo-settings-\(UUID().uuidString)")
        return SettingsFile(url: dir.appendingPathComponent("settings.json"))
    }

    @Test func defaultPathIsInApplicationSupport() {
        #expect(SettingsFile.defaultURL.path.hasSuffix("Library/Application Support/hEARo/settings.json"))
    }

    @Test func fileRoundTripsAndCreatesTheFolder() throws {
        let file = tempFile()
        defer { try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent()) }
        #expect(file.load() == nil, "no file yet")
        var s = FakeEngine.defaultSettings
        s.set("desmack", "range", 12)
        s.setEnabled("deess", false)
        try file.save(s)
        #expect(file.load() == s)
        // Same JSON shape as Rust's settings_to_json.
        let json = try String(contentsOf: file.url, encoding: .utf8)
        #expect(json.contains("\"modules\"") && json.contains("\"enabled\"") && json.contains("\"value\""))
    }

    @Test func corruptFileIsMovedAsideAndDefaultsAreUsed() throws {
        let file = tempFile()
        defer { try? FileManager.default.removeItem(at: file.url.deletingLastPathComponent()) }
        try FileManager.default.createDirectory(at: file.url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("not json {".utf8).write(to: file.url)
        #expect(file.load() == nil)
        #expect(FileManager.default.fileExists(atPath: file.corruptURL.path), "kept for inspection")
        #expect(!FileManager.default.fileExists(atPath: file.url.path))
        let model = ModuleSettingsModel(engine: FakeEngine(), store: file)
        #expect(!model.loadedFromFile && model.settings == FakeEngine.defaultSettings)
    }

    /// Unknown modules/params are ignored, missing ones default, values are clamped (by the engine).
    @Test func tolerantLoadAppliesThroughTheEngine() throws {
        let json =
            #"{"modules":[{"key":"desmack","params":[{"key":"range","value":99},{"key":"future","value":1}]},{"key":"nope"}],"v":2}"#
        let stored = try JSONDecoder().decode(EngineSettings.self, from: Data(json.utf8))
        let engine = FakeEngine()
        let model = ModuleSettingsModel(engine: engine, store: MemorySettingsFile(stored))
        #expect(model.loadedFromFile)
        #expect(engine.calls == ["applySettings"])
        #expect(model.settings.module("desmack")?.value("range") == 48, "clamped to the max")
        #expect(model.settings.module("deess")?.value("range") == 12, "missing → default")
        #expect(model.settings.module("nope") == nil)
    }

    @Test func dragsAreSavedOnceAfterTheLastChange() async {
        let store = MemorySettingsFile()
        let engine = FakeEngine()
        let model = ModuleSettingsModel(engine: engine, store: store, saveDelay: .milliseconds(30))
        for v in stride(from: 0, through: 20, by: 2) { model.set("desmack", "range", Float(v)) }
        #expect(store.saves.isEmpty, "debounced")
        #expect(engine.settings().module("desmack")?.value("range") == 20, "applied live, at once")
        await model.pendingSave?.value
        #expect(store.saves.count == 1)
        #expect(store.saves.last?.module("desmack")?.value("range") == 20)
    }

    @Test func flushSavesAPendingChangeNow() {
        let store = MemorySettingsFile()
        let model = ModuleSettingsModel(engine: FakeEngine(), store: store, saveDelay: .seconds(60))
        model.flush()
        #expect(store.saves.isEmpty, "nothing changed")
        model.set("deess", "range", 3)
        model.flush()
        #expect(store.saves.count == 1 && store.saves.first?.module("deess")?.value("range") == 3)
    }

    @Test func resetRestoresDefaultsButKeepsOnOff() {
        let engine = FakeEngine()
        let model = ModuleSettingsModel(engine: engine, store: MemorySettingsFile())
        model.set("desmack", "range", 6)
        model.set("desmack", "threshold", 30)
        model.set("deess", "range", 3)
        engine.setModuleEnabled("deess", false)
        model.enabledChanged()
        model.reset("desmack")
        #expect(model.settings.module("desmack")?.value("range") == 24)
        #expect(model.settings.module("desmack")?.value("threshold") == 14)
        #expect(model.settings.module("deess")?.value("range") == 3, "other modules untouched")
        model.resetAll()
        #expect(model.settings.module("deess")?.value("range") == 12)
        #expect(model.settings.module("deess")?.enabled == false, "on/off is not a value")
    }

    /// The app: settings are restored at launch, re-applied at start, and toggles are saved too.
    @Test func appModelRestoresAppliesAndSavesToggles() {
        var s = FakeEngine.defaultSettings
        s.set("desmack", "range", 6)
        s.setEnabled("declick", false)
        let store = MemorySettingsFile(s)
        let engine = FakeEngine()
        let model = AppModel(engine: engine, settings: MemoryStore(), settingsFile: store)
        #expect(engine.settings().module("desmack")?.value("range") == 6)
        #expect(model.snapshot.modules.first { $0.id == "declick" }?.enabled == false)
        model.setOn(true)
        #expect(Array(engine.calls.prefix(3)) == ["applySettings", "applySettings", "running=true"])
        model.setModule("deess", enabled: false)
        model.moduleSettings.flush()
        #expect(store.stored?.module("deess")?.enabled == false)
        #expect(store.stored?.module("desmack")?.value("range") == 6)
    }

    /// The older per-module flags still restore on first launch with a file, and migrate into it.
    @Test func legacyFlagsMigrateIntoTheFile() {
        let defaults = MemoryStore()
        defaults.values["module.deess"] = false
        let store = MemorySettingsFile()
        let engine = FakeEngine()
        let model = AppModel(engine: engine, settings: defaults, settingsFile: store)
        #expect(engine.settings().module("deess")?.enabled == false)
        model.moduleSettings.flush()
        #expect(store.stored?.module("deess")?.enabled == false)
    }

    @Test func applyFromTheReviewWindowGoesLiveAndIsRemembered() {
        let defaults = MemoryStore()
        let engine = FakeEngine()
        let model = AppModel(engine: engine, settings: defaults, settingsFile: MemorySettingsFile())
        var trial = model.moduleSettings.settings
        trial.set("desmack", "range", 30)
        trial.setEnabled("dehum", false)
        model.applySettings(trial)
        #expect(engine.calls.last == "applySettings")
        #expect(engine.settings().module("desmack")?.value("range") == 30)
        #expect(defaults.values["module.dehum"] == false)
        #expect(model.snapshot.modules.first { $0.id == "dehum" }?.enabled == false)
    }
}

struct ParamFormatTests {
    let en = Locale(identifier: "en_US")
    private func p(_ unit: String, step: Float = 0, choices: [ParamChoice] = []) -> ParamInfo {
        ParamInfo(id: "x", name: "X", unit: unit, min: -100, max: 20_000, defaultValue: 0, step: step, choices: choices)
    }

    @Test func formatsEveryUnit() {
        #expect(ParamFormat.string(24, p("dB"), locale: en) == "24 dB")
        #expect(ParamFormat.string(-40, p("dB"), locale: en) == "\u{2212}40 dB", "a real minus sign")
        #expect(ParamFormat.string(2.5, p("dB"), locale: en) == "2.5 dB")
        #expect(ParamFormat.string(0.5, p("%"), locale: en) == "50%")
        #expect(ParamFormat.string(1, p("%"), locale: en) == "100%")
        #expect(ParamFormat.string(150, p("Hz"), locale: en) == "150 Hz")
        #expect(ParamFormat.string(7000, p("Hz"), locale: en) == "7 kHz")
        #expect(ParamFormat.string(7500, p("Hz"), locale: en) == "7.5 kHz")
        #expect(ParamFormat.string(60, p("ms"), locale: en) == "60 ms")
        #expect(ParamFormat.string(0.1, p("ms"), locale: en) == "0.1 ms")
        #expect(ParamFormat.string(4, p("ratio"), locale: en) == "4:1")
        #expect(ParamFormat.string(8, p("", step: 1), locale: en) == "8")
        #expect(ParamFormat.string(1.25, p(""), locale: en) == "1.25")
    }

    @Test func choicesShowTheirLabelAndLocaleDecimals() {
        let mains = p("Hz", choices: [ParamChoice(value: 0, label: "Auto"), ParamChoice(value: 50, label: "50 Hz")])
        #expect(ParamFormat.string(0, mains, locale: en) == "Auto")
        #expect(ParamFormat.string(49, mains, locale: en) == "50 Hz", "nearest choice")
        #expect(ParamFormat.string(7500, p("Hz"), locale: Locale(identifier: "sv_SE")) == "7,5 kHz")
    }

    /// De-click's primary control is a sensitivity (a repair has no depth); the rest are depths.
    @Test func primaryControlsAreLabelledByWhatTheyDo() {
        let cat = FakeEngine.catalog
        #expect(cat.first { $0.id == "declick" }?.primaryParam?.name == "Sensitivity")
        #expect(cat.first { $0.id == "desmack" }?.primaryParam?.unit == "dB")
    }
}
