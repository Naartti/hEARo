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

    /// Every route into the window selects the tab first, then opens the window and activates
    /// the menu-bar-only app (otherwise the window opens behind the frontmost app).
    @Test func openMainWindowSelectsTheTabOpensAndActivates() {
        let model = AppModel(engine: FakeEngine(.previewRunning))
        var log: [String] = []
        model.openMainWindow(.settings, open: { log.append("open \($0) on \(model.mainTab)") }, activate: { log.append("activate") })
        #expect(log == ["open main on settings", "activate"])
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
