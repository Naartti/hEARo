import Foundation
import Testing

@testable import HearoKit

@MainActor
struct AppModelTests {
    let en = Locale(identifier: "en_US")

    @Test func statusLineWhenProcessing() {
        let model = AppModel(engine: FakeEngine(.previewRunning), locale: en)
        #expect(model.statusLine == "Processing 3 apps · +18 ms · 1.4% CPU")
    }

    @Test func statusLineUsesTheLocaleDecimalSeparator() {
        let model = AppModel(engine: FakeEngine(.previewRunning), locale: Locale(identifier: "sv_SE"))
        #expect(model.statusLine.contains("1,4% CPU"))
    }

    @Test func statusLineWhenIdleOffOrDenied() {
        var idle = EngineSnapshot.previewRunning
        idle.idle = true
        #expect(AppModel(engine: FakeEngine(idle), locale: en).statusLine == "Waiting for audio · +18 ms")
        #expect(AppModel(engine: FakeEngine()).statusLine == "Off")
        #expect(AppModel(engine: FakeEngine(.previewDenied)).statusLine.contains("permission"))
    }

    /// hEARo follows the system output; the popover says where the sound goes, or why it's
    /// paused (audio then plays normally, unprocessed).
    @Test func outputLineShowsTheDeviceOrTheProblem() {
        #expect(AppModel(engine: FakeEngine(.previewRunning)).outputLine == "Output: MacBook Pro Speakers")
        var paused = EngineSnapshot()
        paused.problem = "aggregate device failed"
        #expect(AppModel(engine: FakeEngine(paused)).outputLine == "Paused, playing unprocessed: aggregate device failed")
        #expect(AppModel(engine: FakeEngine()).outputLine == nil)
    }

    @Test func singularApp() {
        var s = EngineSnapshot.previewRunning
        s.processingApps = 1
        #expect(AppModel(engine: FakeEngine(s), locale: en).statusLine.hasPrefix("Processing 1 app ·"))
    }

    @Test func togglesCallTheEngine() {
        let engine = FakeEngine()
        let model = AppModel(engine: engine)
        model.setOn(true)
        model.setModule("deess", enabled: false)
        model.setBypass(true)
        #expect(engine.calls == ["running=true", "deess=false", "bypass=true"])
        #expect(model.snapshot.modules.first { $0.id == "deess" }?.enabled == false)
        #expect(model.statusLine.hasSuffix("· bypassed"))
    }

    @Test func startFailureIsShownNotThrown() {
        let engine = FakeEngine()
        engine.startError = FakeEngine.FakeError.notRunning
        let model = AppModel(engine: engine)
        model.setOn(true)
        #expect(!model.isOn)
        #expect(model.errorMessage != nil)
        #expect(model.iconState == .problem)
    }

    @Test func iconStatesFollowTheEngine() {
        #expect(AppModel(engine: FakeEngine()).iconState == .off)
        #expect(AppModel(engine: FakeEngine(.previewRunning)).iconState == .on)
        #expect(AppModel(engine: FakeEngine(.previewDenied)).iconState == .problem)
    }

    @Test func removalGlowsThenSettles() {
        let engine = FakeEngine(.previewRunning)
        let model = AppModel(engine: engine)
        let t0 = Date()
        var s = engine.state
        s.modules[0].triggers += 1
        engine.set(s)
        model.refresh(now: t0)
        #expect(model.iconState == .removing)
        #expect(model.iconState.isAccent)
        model.refresh(now: t0.addingTimeInterval(0.6))
        #expect(model.iconState == .on)
    }

    @Test func snapshotShowsCheckmarkAndCounts() {
        let engine = FakeEngine(.previewRunning)
        let model = AppModel(engine: engine)
        let t0 = Date()
        #expect(model.saveSnapshot(now: t0))
        #expect(model.iconState == .snapshotSaved)
        #expect(model.snapshot.snapshotsSaved == 1)
        // A removal during the checkmark doesn't override it.
        var s = engine.state
        s.modules[0].triggers += 5
        engine.set(s)
        model.refresh(now: t0.addingTimeInterval(0.3))
        #expect(model.iconState == .snapshotSaved)
        model.refresh(now: t0.addingTimeInterval(1.3))
        #expect(model.iconState == .on)
    }

    @Test func snapshotWhileOffReportsError() {
        let model = AppModel(engine: FakeEngine())
        #expect(!model.saveSnapshot())
        #expect(model.errorMessage?.contains("Snapshot failed") == true)
    }

    @Test func waveformBufferKeepsTheNewestColumns() {
        var b = WaveformBuffer(capacity: 3)
        b.append(WaveformChunk(next: 2, peaks: [0.1, 0.2], removed: [false, true]))
        b.append(WaveformChunk(next: 4, peaks: [0.3, 0.4], removed: [false, false]))
        #expect(b.columns.map(\.peak) == [0.2, 0.3, 0.4])
        #expect(b.columns.first?.removed == true)
    }

    @Test func waveformHeightMapsDecibels() {
        #expect(WaveformBuffer.height(1) == 1)
        #expect(WaveformBuffer.height(0) == 0)
        #expect(abs(WaveformBuffer.height(0.04467) - 0.5) < 0.01)  // −27 dBFS → middle of −54…0
        #expect(WaveformBuffer.height(0.0001) == 0)
    }

    @Test func pollingAppendsAndAdvancesTheCursor() {
        let engine = FakeEngine(.previewRunning)
        let model = AppModel(engine: engine)
        engine.nextWaveform = WaveformChunk(next: 5, peaks: [0.5, 0.5, 0.5], removed: [false, true, false])
        model.pollWaveform()
        #expect(model.waveform.columns.count == 3)
        #expect(model.waveform.columns[1].removed)
        // An engine restart (cursor goes back) clears the old waveform.
        engine.nextWaveform = WaveformChunk(next: 1, peaks: [0.1], removed: [false])
        model.pollWaveform()
        #expect(model.waveform.columns.count == 1)
    }

    @Test func exportAndImportReportResults() {
        let engine = FakeEngine(.previewRunning)
        let model = AppModel(engine: engine)
        model.exportSnapshots(to: URL(fileURLWithPath: "/tmp/x.zip"))
        #expect(engine.exported.count == 1)
        #expect(model.infoMessage?.contains("x.zip") == true)
        engine.importResult = SnapshotImport(added: 4, skipped: 2, renamed: 1)
        model.importSnapshots(from: URL(fileURLWithPath: "/tmp/y.zip"))
        #expect(model.infoMessage == "Imported snapshots: 4 added, 2 already here, 1 renamed")
    }

    @Test func launchAtLoginGoesThroughTheLoginItem() {
        let item = FakeLoginItem()
        let model = AppModel(engine: FakeEngine(), loginItem: item)
        #expect(model.canLaunchAtLogin && !model.launchAtLogin)
        model.setLaunchAtLogin(true)
        #expect(item.isEnabled && model.launchAtLogin)
        #expect(!AppModel(engine: FakeEngine()).canLaunchAtLogin)
    }

    @Test func choicesAreRememberedAcrossLaunches() {
        let store = MemoryStore()
        let first = AppModel(engine: FakeEngine(), settings: store)
        #expect(first.shouldStartAtLaunch, "default: start processing")
        first.setModule("deess", enabled: false)
        first.setOn(false)
        // "Relaunch": a fresh engine with defaults, same store.
        let engine = FakeEngine()
        let second = AppModel(engine: engine, settings: store)
        #expect(!second.shouldStartAtLaunch)
        #expect(engine.state.modules.first { $0.id == "deess" }?.enabled == false)
        #expect(engine.state.modules.first { $0.id == "dehum" }?.enabled == true)
    }
}

/// The ⋯ menu and the right-click menu on the menu bar icon share these entries.
@MainActor
struct AppMenuTests {
    @Test func sameEntriesWithAboutFirstAndQuitLast() {
        let menu = AppMenu(model: AppModel(engine: FakeEngine(.previewRunning)), show: { _ in })
        #expect(menu.items == [.about, .divider, .window, .review, .advanced, .divider, .export, .import, .showInFinder, .divider, .quit])
        #expect(menu.nsMenu().items.map(\.title) == [
            "About hEARo", "", "Open hEARo Window", "Review snapshots…", "Advanced settings…", "", "Export snapshots…",
            "Import snapshots…", "Show snapshots in Finder", "", "Quit hEARo",
        ])
        #expect(menu.nsMenu().items.first?.keyEquivalent == "")
        #expect(menu.nsMenu().items.last?.keyEquivalent == "q")
    }

    @Test func aboutVersionShowsBuildShaAndDebugOnlyForDebugBuilds() {
        let debug: [String: Any] = [
            "CFBundleVersion": "123", "HearoGitSHA": "abc1234", "HearoBuildConfiguration": "debug",
        ]
        #expect(AboutInfo.version(info: debug) == "123 · abc1234 · debug")
        var release = debug
        release["HearoBuildConfiguration"] = "release"
        #expect(AboutInfo.version(info: release) == "123 · abc1234")
        #expect(AboutInfo.version(info: [:]) == "dev")
    }

    @Test func aboutCreditsDescribeTheAppAndLicence() {
        let credits = AboutInfo.credits().string
        #expect(credits.contains("clicks"))
        #expect(credits.contains("MIT OR Apache-2.0"))
        #expect(credits.contains("never sent anywhere"))
    }

    /// Window, Review and Advanced all open the one hEARo window, each on its own tab.
    @Test func windowEntriesOpenTheMainWindowOnTheirTab() {
        var shown: [MainTab] = []
        let menu = AppMenu(model: AppModel(engine: FakeEngine(.previewRunning)), show: { shown.append($0) })
        menu.perform(.window)
        menu.perform(.review)
        menu.perform(.advanced)
        #expect(shown == [.controls, .review, .settings])
        #expect(AppMenu.Item.advanced.title == "Advanced settings…")
    }
}
