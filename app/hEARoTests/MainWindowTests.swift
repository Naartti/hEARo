import AppKit
import Foundation
import Testing

@testable import HearoKit

/// The one hEARo window: Controls (the popover's content, kept open), Review, Settings tabs.
@MainActor
struct MainWindowTests {
    @Test func tabsInOrderWithTitlesAndShortcuts() {
        #expect(MainTab.allCases == [.controls, .review, .settings])
        #expect(MainTab.allCases.map(\.title) == ["Controls", "Review", "Settings"])
        #expect(MainTab.allCases.map(\.shortcut) == ["1", "2", "3"])
    }

    @Test func showSelectsTheTab() {
        let model = AppModel(engine: FakeEngine(.previewRunning))
        #expect(model.mainTab == .controls)
        model.show(.review)
        #expect(model.mainTab == .review)
    }

    /// The popover and the window's Controls tab can both show the live waveform; it keeps
    /// polling until the last of them goes away, and never polls while neither is visible.
    @Test func waveformPollsWhileAnyLiveViewIsVisible() {
        let model = AppModel(engine: FakeEngine(.previewRunning))
        #expect(!model.isPollingWaveform)
        model.setLiveViewVisible(true, viewer: .popover)
        model.setLiveViewVisible(true, viewer: .window)
        model.setLiveViewVisible(false, viewer: .popover)
        #expect(model.isPollingWaveform)
        model.setLiveViewVisible(false, viewer: .window)
        #expect(!model.isPollingWaveform)
        model.setLiveViewVisible(false, viewer: .window)  // a stray disappear is harmless
        #expect(!model.isPollingWaveform)
    }

    /// A DockPresence that logs what it does to the app instead of doing it; `later` work runs
    /// only when the test calls `runLater()`.
    @MainActor
    final class DockLog {
        var entries: [String] = []
        var pending: [@MainActor () -> Void] = []
        lazy var dock = DockPresence(
            platform: .init(
                setPolicy: { [unowned self] in entries.append($0 == .regular ? "regular" : "accessory") },
                restoreFocus: { [unowned self] in entries.append("restore focus") },
                later: { [unowned self] in pending.append($0) }))

        func runLater() {
            let work = pending
            pending = []
            work.forEach { $0() }
        }
    }

    /// Every route into the window selects the tab first, makes hEARo a regular app (Dock icon,
    /// ⌘Tab, menu bar), then opens the window and activates the app (otherwise the window opens
    /// behind the frontmost app). The policy changes before activation so the menu bar appears.
    @Test func openMainWindowSelectsTheTabShowsInDockOpensAndActivates() {
        let log = DockLog()
        let model = AppModel(engine: FakeEngine(.previewRunning))
        model.dock = log.dock
        model.openMainWindow(.settings, open: { log.entries.append("open \($0) on \(model.mainTab)") }, activate: { log.entries.append("activate") })
        #expect(log.entries == ["regular", "open main on settings", "activate"])
    }

    /// The window itself puts hEARo in the Dock when it appears, so a route that skips
    /// `openMainWindow` (state restoration, the Window menu) still gets the Dock icon.
    @Test func theWindowAppearingPutsHearoInTheDock() {
        let log = DockLog()
        log.dock.windowAppeared()
        #expect(log.entries == ["regular"])
        #expect(log.dock.isWindowOpen)
    }

    /// Closing the window takes hEARo back to menu-bar-only (no Dock icon, not in ⌘Tab) after
    /// the close finishes, and hands focus back to the app the user was in.
    @Test func closingTheWindowLeavesTheDockAndReturnsFocus() {
        let log = DockLog()
        log.dock.windowAppeared()
        log.dock.windowClosed()
        #expect(!log.dock.isWindowOpen)
        #expect(log.entries == ["regular"])  // not inside willClose
        log.runLater()
        #expect(log.entries == ["regular", "accessory", "restore focus"])
    }

    /// Closed and reopened before the deferred switch runs: hEARo stays in the Dock.
    @Test func reopeningBeforeTheCloseSettlesKeepsTheDockIcon() {
        let log = DockLog()
        log.dock.windowAppeared()
        log.dock.windowClosed()
        log.dock.windowAppeared()
        log.runLater()
        #expect(log.entries == ["regular", "regular"])
        #expect(log.dock.isWindowOpen)
    }

    /// The menu bar's tab commands (View ⌘1–⌘3, Settings… ⌘,) just switch tabs while the window
    /// is open, and open the window on that tab otherwise.
    @Test func tabCommandsSwitchInsideTheWindowAndOpenItOtherwise() {
        let log = DockLog()
        let model = AppModel(engine: FakeEngine(.previewRunning))
        model.dock = log.dock
        var opened: [String] = []
        model.tabCommand(.review, open: { opened.append($0) }, activate: {})
        #expect(opened == ["main"] && model.mainTab == .review)
        log.dock.windowAppeared()
        model.tabCommand(.settings, open: { opened.append($0) }, activate: {})
        #expect(opened == ["main"] && model.mainTab == .settings)
    }

    /// The View menu lists the tabs in toolbar order with ⌘1–⌘3; Settings… is ⌘, → Settings.
    @Test func menuBarCommandItems() {
        #expect(MainWindowCommands.items.map(\.title) == ["Settings…", "Controls", "Review", "Settings"])
        #expect(MainWindowCommands.items.map(\.tab) == [.settings, .controls, .review, .settings])
        #expect(MainWindowCommands.items.map(\.shortcut) == [",", "1", "2", "3"])
    }

    /// A second live view joining doesn't restart the poll; Reduce Motion in either slows it.
    @Test func waveformPollIntervalFollowsReduceMotion() {
        let model = AppModel(engine: FakeEngine(.previewRunning))
        model.setLiveViewVisible(true, viewer: .window)
        #expect(model.waveformPollInterval == 1.0 / 30)
        model.setLiveViewVisible(true, viewer: .popover, reduceMotion: true)
        #expect(model.waveformPollInterval == 0.1)
        model.setLiveViewVisible(true, viewer: .popover, reduceMotion: false)
        #expect(model.waveformPollInterval == 1.0 / 30)
        model.setLiveViewVisible(false, viewer: .popover)
        model.setLiveViewVisible(false, viewer: .window)
        #expect(model.waveformPollInterval == nil)
    }

    /// Inside the window the ⋯ menu has no "Open hEARo Window" (it would do nothing there).
    @Test func menuInsideTheWindowLeavesOutOpenWindow() {
        let model = AppModel(engine: FakeEngine(.previewRunning))
        #expect(AppMenu(model: model, show: { _ in }).items.contains(.window))
        #expect(!AppMenu(model: model, inWindow: true, show: { _ in }).items.contains(.window))
    }

    /// Leaving the Review tab only stops playback; the open clip's render survives. Closing the
    /// window drops renders like the old review window did.
    @Test func reviewTabHiddenKeepsStateWindowClosedDropsIt() {
        let player = FakePlayer()
        let review = ReviewModel(
            library: SnapshotLibrary(folder: URL(fileURLWithPath: "/nonexistent")), player: player,
            live: AppModel(engine: FakeEngine(.previewRunning)), renderer: FakeClipRenderer())
        let url = URL(fileURLWithPath: "/nonexistent/snapshot-1763000000-missed.wav")
        review.inspector?.showPreview(.preview(for: AudioClip.synthetic()), clip: url)
        player.isPlaying = true
        review.tabHidden()
        #expect(!player.isPlaying)
        #expect(review.inspector?.result != nil)
        review.windowClosed()
        #expect(review.inspector?.result == nil)
    }
}
