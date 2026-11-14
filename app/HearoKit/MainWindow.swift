import AppKit
import SwiftUI

/// Scene id of the hEARo window (`Window(..., id:)` in the app).
public let mainWindowID = "main"

/// The hEARo window's tabs, in toolbar order.
public enum MainTab: String, CaseIterable, Identifiable, Sendable {
    /// The popover's controls, kept open (human request 2026-11-14: handy for validation/testing).
    case controls
    /// Snapshot review & annotation (`ReviewView`).
    case review
    /// Every parameter of every module (`AdvancedSettingsView`).
    case settings

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .controls: "Controls"
        case .review: "Review"
        case .settings: "Settings"
        }
    }

    /// The window size this tab opens at (each tab remembers its own size while the app runs,
    /// like Xcode's settings tabs: a narrow Controls window can sit beside the app under test).
    public var defaultSize: CGSize {
        switch self {
        case .controls: CGSize(width: 460, height: 640)
        case .review: CGSize(width: 1300, height: 680)
        case .settings: CGSize(width: 600, height: 720)
        }
    }

    /// ⌘1, ⌘2, ⌘3 (like Finder's view modes).
    public var shortcut: Character {
        switch self {
        case .controls: "1"
        case .review: "2"
        case .settings: "3"
        }
    }
}

/// A view that shows the live waveform; it polls while any of them is on screen.
public enum LiveViewer: Hashable, Sendable {
    case popover, window
}

extension AppModel {
    /// Open (or bring forward) the hEARo window on `tab`. hEARo is menu-bar-only (LSUIElement)
    /// until then; while the window is open it is a regular app — Dock icon, ⌘Tab, its own menu
    /// bar (human request 2026-11-14) — and it is activated, or the window would open behind
    /// the frontmost app.
    public func openMainWindow(_ tab: MainTab, using openWindow: OpenWindowAction) {
        openMainWindow(tab, open: { openWindow(id: $0) })
    }

    /// Testable core of `openMainWindow(_:using:)`: select, show in the Dock, open, activate —
    /// in that order (the policy must change before activation, or the menu bar doesn't appear).
    func openMainWindow(
        _ tab: MainTab, setPolicy: (NSApplication.ActivationPolicy) -> Void = { _ = NSApp?.setActivationPolicy($0) },
        open: (String) -> Void, activate: () -> Void = { NSApp?.activate() }
    ) {
        show(tab)
        setPolicy(.regular)
        open(mainWindowID)
        activate()
    }

    /// The hEARo window closed: back to menu-bar-only (no Dock icon, not in ⌘Tab).
    func mainWindowClosed(setPolicy: (NSApplication.ActivationPolicy) -> Void = { _ = NSApp?.setActivationPolicy($0) }) {
        setPolicy(.accessory)
    }
}

/// The menu bar hEARo shows while its window is open (and so it is a regular app): a View menu
/// with the tabs (⌘1–⌘3) and the app menu's Settings… (⌘,) opening the Settings tab. Each item
/// opens the window on its tab, so it works whether or not the window is key.
public struct MainWindowCommands: Commands {
    /// The View menu's tab items, in toolbar order.
    static let viewItems = MainTab.allCases
    /// The tab the app menu's Settings… opens.
    static let settingsTab = MainTab.settings

    let model: AppModel

    public init(model: AppModel) { self.model = model }

    public var body: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button(AppMenu.Item.about.title) { AppMenu.showAbout() }
        }
        CommandGroup(replacing: .appSettings) {
            OpenTabButton(model: model, tab: Self.settingsTab, title: "Settings…", shortcut: ",")
        }
        CommandGroup(before: .toolbar) {
            ForEach(Self.viewItems) { tab in
                OpenTabButton(model: model, tab: tab, title: tab.title, shortcut: tab.shortcut)
            }
            Divider()
        }
    }
}

/// A menu command that opens the hEARo window on a tab (a view, for the scene's `openWindow`).
private struct OpenTabButton: View {
    let model: AppModel
    let tab: MainTab
    let title: String
    let shortcut: Character
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Button(title) { model.openMainWindow(tab, using: openWindow) }
            .keyboardShortcut(KeyEquivalent(shortcut), modifiers: .command)
    }
}

private struct LiveViewOnScreenKey: EnvironmentKey {
    static let defaultValue = true
}

extension EnvironmentValues {
    /// False while the window holding a live view is minimised, hidden or fully covered, so the
    /// view stops polling (`ControlsView`).
    public var liveViewOnScreen: Bool {
        get { self[LiveViewOnScreenKey.self] }
        set { self[LiveViewOnScreenKey.self] = newValue }
    }
}

/// The one hEARo window: Controls · Review · Settings as a segmented control in the toolbar
/// (the Activity Monitor pattern). It stays open until closed, unlike the popover.
public struct MainWindowView: View {
    @Bindable var model: AppModel
    let review: ReviewModel
    @State private var window: NSWindow?
    @State private var onScreen = true
    /// Each tab's window size, remembered while the app runs.
    @State private var sizes: [MainTab: CGSize] = [:]
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(model: AppModel, review: ReviewModel) {
        self.model = model
        self.review = review
    }

    public var body: some View {
        content
            .environment(\.liveViewOnScreen, onScreen)
            .background(
                WindowObserver(
                    attached: { window = $0 }, onScreen: { onScreen = $0 },
                    willClose: {
                        review.windowClosed()
                        model.mainWindowClosed()
                    }))
            .onChange(of: model.mainTab) { old, new in resize(from: old, to: new) }
            .navigationTitle("hEARo")
            // The tabs are the toolbar; the title text would crowd them into an overflow menu
            // in the narrow Controls window (the window keeps "hEARo" for Mission Control/VoiceOver).
            .toolbar(removing: .title)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Picker("View", selection: $model.mainTab) {
                        ForEach(MainTab.allCases) { tab in
                            Text(tab.title).tag(tab)
                        }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                    .accessibilityLabel("hEARo section")
                    // ⌘1–⌘3 live in the View menu (`MainWindowCommands`).
                    .help("Controls ⌘1 · Review ⌘2 · Settings ⌘3")
                }
            }
    }

    @ViewBuilder private var content: some View {
        switch model.mainTab {
        case .controls:
            ScrollView {
                ControlsView(model: model, viewer: .window)
                    .frame(maxWidth: .infinity)
            }
            .frame(minWidth: 420, minHeight: 460)
        case .review:
            ReviewView(model: review, locale: model.locale)
        case .settings:
            // A wide window (sized for Review) would stretch the sliders; keep a readable column.
            AdvancedSettingsView(model: model)
                .frame(maxWidth: 680)
                .frame(maxWidth: .infinity)
        }
    }

    /// Give each tab its own window size, keeping the top-left corner where it is. Deferred a
    /// run-loop turn so SwiftUI has applied the new tab's minimum size first.
    private func resize(from old: MainTab, to new: MainTab) {
        guard let window else { return }
        sizes[old] = window.frame.size
        let target = sizes[new] ?? new.defaultSize
        let animate = !reduceMotion
        DispatchQueue.main.async {
            let min = window.minSize
            let size = CGSize(width: max(target.width, min.width), height: max(target.height, min.height))
            var frame = window.frame
            frame.origin.y += frame.height - size.height
            frame.size = size
            window.setFrame(frame, display: true, animate: animate)
        }
    }
}

/// Reports the hosting `NSWindow`, whether it is on screen (not minimised, hidden or fully
/// covered — AppKit's occlusion state) and when it closes.
struct WindowObserver: NSViewRepresentable {
    let attached: (NSWindow) -> Void
    let onScreen: (Bool) -> Void
    let willClose: () -> Void

    func makeNSView(context: Context) -> ObserverView {
        ObserverView(attached: attached, onScreen: onScreen, willClose: willClose)
    }

    func updateNSView(_ view: ObserverView, context: Context) {}

    final class ObserverView: NSView {
        let attached: (NSWindow) -> Void
        let onScreen: (Bool) -> Void
        let willClose: () -> Void
        private var tokens: [NSObjectProtocol] = []

        init(attached: @escaping (NSWindow) -> Void, onScreen: @escaping (Bool) -> Void, willClose: @escaping () -> Void) {
            self.attached = attached
            self.onScreen = onScreen
            self.willClose = willClose
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("not used") }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            tokens.forEach(NotificationCenter.default.removeObserver)
            tokens = []
            guard let window else { return }
            attached(window)
            let center = NotificationCenter.default
            tokens.append(
                center.addObserver(forName: NSWindow.didChangeOcclusionStateNotification, object: window, queue: .main) {
                    [weak self] _ in
                    MainActor.assumeIsolated {
                        guard let self, let window = self.window else { return }
                        self.onScreen(window.occlusionState.contains(.visible))
                    }
                })
            tokens.append(
                center.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.willClose() }
                })
        }

        isolated deinit {
            tokens.forEach(NotificationCenter.default.removeObserver)
        }
    }
}

// PreviewProvider rather than #Preview (no macro plugin with Command Line Tools).
struct MainWindowView_Previews: PreviewProvider {
    static var previews: some View {
        let model = AppModel(engine: FakeEngine(.previewRunning))
        MainWindowView(model: model, review: .preview(live: model))
            .frame(width: 1100, height: 640)
    }
}
