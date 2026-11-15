import HearoKit
import HearoLive
import SwiftUI

/// Entry point. Menu-bar only (LSUIElement) until the hEARo window opens, then a regular app with
/// a Dock icon until it closes (`openMainWindow`); all UI lives in HearoKit.
@main
struct HearoApp: App {
    @State private var model: AppModel
    @State private var hotKey: GlobalHotKey?
    @State private var review: ReviewModel

    init() {
        // Preferences saved under the old bundle id (dev.hearo.app) carry over once.
        DefaultsMigration.runForApp()
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
            ControlsView(model: model, viewer: .popover)
        } label: {
            // The icon; right-click shows the app menu (window, review, export/import, quit).
            MenuBarLabel(model: model)
        }
        .menuBarExtraStyle(.window)

        // The one window: Controls (the popover kept open) · Review snapshots · Settings, as
        // toolbar tabs. Opened from the popover or the menus, never at launch. A plain `Window`,
        // not the `Settings` scene: settings are a tab of this one window (app menu Settings… ⌘,).
        Window("hEARo", id: mainWindowID) {
            MainWindowView(model: model, review: review)
        }
        // Opens at the size of the tab it opens on (Controls is narrow, Review wide).
        .defaultWindowPlacement { _, _ in WindowPlacement(size: model.mainTab.defaultSize) }
        .windowResizability(.contentMinSize)
        .defaultLaunchBehavior(.suppressed)
        // Never reopened at launch, even if it was open at quit (launch stays menu-bar-only).
        .restorationBehavior(.disabled)
        // Shown while the window is open (hEARo is then a regular app with a Dock icon).
        .commands { MainWindowCommands(model: model) }
    }
}
