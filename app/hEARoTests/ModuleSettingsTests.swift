import Foundation
import Testing

@testable import HearoKit

@MainActor
struct ModuleSettingsTests {
    @Test func catalogHasOnePrimaryPerCleanupModule() {
        let cat = FakeEngine().moduleCatalog()
        #expect(cat.filter(\.cleanup).map(\.id) == cleanupModuleKeys)
        for m in cat where m.cleanup { #expect(m.params.filter(\.primary).count == 1, "\(m.id)") }
        #expect(cat.last?.alwaysOn == true && cat.last?.primaryParam == nil)
    }

    @Test func settingsJSONMatchesTheRustShapeAndIsTolerant() throws {
        let json = #"{"modules":[{"key":"desmack","params":[{"key":"range","value":12}],"future":1},{"key":"deess"}]}"#
        let s = try JSONDecoder().decode(EngineSettings.self, from: Data(json.utf8))
        #expect(s.module("desmack")?.value("range") == 12)
        #expect(s.module("desmack")?.enabled == true, "missing enabled = on")
        #expect(s.module("deess")?.params.isEmpty == true)
        let back = try JSONDecoder().decode(EngineSettings.self, from: JSONEncoder().encode(s))
        #expect(back == s)
        #expect(try JSONDecoder().decode(EngineSettings.self, from: Data("{}".utf8)) == EngineSettings())
    }

    @Test func fakeEngineClampsSnapsAndRestores() {
        let e = FakeEngine()
        e.setParam("desmack", "range", 99)
        e.setParam("dehum", "mains", 49)
        e.setParam("nope", "range", 1)
        let s = e.settings()
        #expect(s.module("desmack")?.value("range") == 48)
        #expect(s.module("dehum")?.value("mains") == 50)
        e.applySettings(EngineSettings())
        #expect(e.settings().module("desmack")?.value("range") == 24, "missing = defaults")
        e.applySettings(s)
        #expect(e.settings() == s)
    }

    @Test func waveformBufferKeepsOutputPeaksAndReductions() {
        var b = WaveformBuffer(capacity: 10)
        b.append(FakeEngine.syntheticWaveform(columns: 3, start: 39))
        #expect(b.columns.count == 3)
        let smack = b.columns[1]  // column 40
        #expect(smack.removed && smack.reductionDb.count == cleanupModuleKeys.count)
        #expect(smack.reductionDb[3] == 18 && smack.outPeak < smack.peak)
        // Older producers without the new fields still work.
        b.append(WaveformChunk(next: 1, peaks: [0.2], removed: [false]))
        #expect(b.columns.last?.outPeak == 0.2 && b.columns.last?.reductionDb == [])
    }

    @Test func fakeRenderZeroesBypassedAndDisabledModules() throws {
        let dir = FileManager.default.temporaryDirectory
        let input = dir.appendingPathComponent("hearo-fake-in-\(UUID().uuidString).wav")
        let output = dir.appendingPathComponent("hearo-fake-out-\(UUID().uuidString).wav")
        try Data([1, 2, 3]).write(to: input)
        defer { try? FileManager.default.removeItem(at: input); try? FileManager.default.removeItem(at: output) }
        var s = FakeEngine.defaultSettings
        s.setEnabled("desmack", false)
        let r = try FakeEngine().renderer.render(input, to: output, settings: s, bypass: false, columnsPerSecond: 100)
        #expect(r.inputPeaks.count == 800 && r.reductions.map(\.key) == cleanupModuleKeys)
        #expect(r.reductions[3].gainReductionDb.allSatisfy { $0 == 0 })
        #expect(r.reductions[4].gainReductionDb.contains { $0 > 0 })
        let b = try FakeEngine().renderer.render(input, to: output, settings: s, bypass: true, columnsPerSecond: 60)
        #expect(b.reductions.allSatisfy { $0.gainReductionDb.allSatisfy { $0 == 0 } })
        #expect(FileManager.default.fileExists(atPath: output.path))
    }
}
