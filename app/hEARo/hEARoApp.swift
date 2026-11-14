import HearoKit
import HearoLive
import SwiftUI

/// Entry point. Menu-bar only (LSUIElement); all UI lives in HearoKit.
@main
struct HearoApp: App {
    @State private var model: AppModel
    @State private var hotKey: GlobalHotKey?
    @State private var review: ReviewModel

    init() {
        // Module settings (amounts, advanced parameters, on/off) persist in
        // ~/Library/Application Support/hEARo/settings.json and are applied before starting.
        let model = AppModel(
            engine: RustEngine(), loginItem: SystemLoginItem(), settings: DefaultsStore(), settingsFile: SettingsFile())
        _model = State(initialValue: model)
        // The review window renders clips with trial settings (its inspector) through the engine.
        _review = State(
            initialValue: ReviewModel(
                library: SnapshotLibrary(folder: model.snapshotFolder), player: ClipPlayer(), live: model,
                renderer: model.renderer))
        // Start processing at launch (unless the user switched it off last time) and poll the
        // engine (drives status and icon states).
        if model.shouldStartAtLaunch { model.setOn(true) }
        model.startPolling()
        // A slider moved just before logout/restart/quit is still saved (saves are debounced).
        NotificationCenter.default.addObserver(forName: NSApplication.willTerminateNotification, object: nil, queue: .main) {
            _ in MainActor.assumeIsolated { model.moduleSettings.flush() }
        }
        _hotKey = State(initialValue: GlobalHotKey { model.saveSnapshot() })
    }

    var body: some Scene {
        MenuBarExtra {
            PopoverView(model: model)
        } label: {
            // The icon; right-click shows the app menu (review, export/import, quit).
            MenuBarLabel(model: model, reviewWindowID: reviewWindowID)
        }
        .menuBarExtraStyle(.window)

        // Snapshot review & annotation (M6). Opened from the popover's ⋯ menu, never at launch.
        Window("Review snapshots", id: reviewWindowID) {
            ReviewView(model: review, locale: model.locale)
        }
        .defaultSize(width: 1300, height: 640)
        .defaultLaunchBehavior(.suppressed)

        // Every parameter of every module. A plain `Window` (not the `Settings` scene): an
        // LSUIElement app has no app menu to open Settings from, and `openWindow` + activate
        // brings it to the front reliably, like the review window.
        Window("Advanced Settings", id: settingsWindowID) {
            AdvancedSettingsView(model: model)
        }
        .defaultSize(width: 540, height: 680)
        .defaultLaunchBehavior(.suppressed)
    }
}
