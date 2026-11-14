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
        #expect(MainTab.allCases.allSatisfy { !$0.systemImage.isEmpty })
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
}
