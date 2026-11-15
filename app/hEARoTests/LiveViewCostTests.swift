import AppKit
import Foundation
import SwiftUI
import Testing

@testable import HearoKit

/// What the live waveform costs on the main thread while the controls are on screen.
///
/// Manual measurement, off by default (timing depends on the machine): run with
/// `HEARO_PERF=1 swift test --filter LiveViewCost` from `app/`. It hosts the real
/// `ControlsView` in a window, polls the fake engine at the live rate and lets SwiftUI update and
/// draw after every poll, then prints main-thread CPU per second of live view.
@MainActor
struct LiveViewCostTests {
    nonisolated static var enabled: Bool { ProcessInfo.processInfo.environment["HEARO_PERF"] == "1" }

    private func threadCPU() -> Double {
        var ts = timespec()
        clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts)
        return Double(ts.tv_sec) + Double(ts.tv_nsec) / 1e9
    }

    @Test(.enabled(if: LiveViewCostTests.enabled)) func controlsViewCostPerSecond() {
        let model = AppModel(engine: FakeEngine())
        model.setOn(true)
        model.refresh()
        let host = NSHostingView(rootView: ControlsView(model: model, viewer: .window))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 360, height: 640), styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.orderOut(nil) }

        func frame() {
            model.pollWaveform()
            RunLoop.current.run(until: Date())  // let SwiftUI apply the change
            host.layoutSubtreeIfNeeded()
            host.displayIfNeeded()
        }
        for _ in 0..<30 { frame() }  // warm up
        let frames = 300  // 10 s at the 30 Hz live rate
        let start = threadCPU()
        for i in 0..<frames {
            frame()
            if i % 6 == 0 { model.refresh() }  // the 5 Hz status poll
        }
        let cpu = threadCPU() - start
        let perSecond = cpu / (Double(frames) / 30)
        print("LIVE VIEW COST: \(String(format: "%.1f", cpu / Double(frames) * 1000)) ms/frame, "
            + "\(String(format: "%.1f", perSecond * 100)) % of a core at 30 Hz")
    }
}
