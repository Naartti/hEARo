import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The app menu (about, the hEARo window and its tabs, export/import, Finder, launch at login, quit). The same entries back
/// the popover's ⋯ menu and the right-click menu on the menu bar icon, so they can't drift apart.
@MainActor
public struct AppMenu {
    public enum Item: Equatable {
        case about, window, review, advanced, export, `import`, showInFinder, launchAtLogin(on: Bool), quit, divider

        public var title: String {
            switch self {
            case .about: "About hEARo"
            case .window: "Open hEARo Window"
            case .review: "Review snapshots…"
            case .advanced: "Advanced settings…"
            case .export: "Export snapshots…"
            case .import: "Import snapshots…"
            case .showInFinder: "Show snapshots in Finder"
            case .launchAtLogin: "Launch at login"
            case .quit: "Quit hEARo"
            case .divider: ""
            }
        }
    }

    let model: AppModel
    /// Opens the hEARo window on a tab (`MainWindowView`).
    let show: (MainTab) -> Void

    public init(model: AppModel, show: @escaping (MainTab) -> Void) {
        self.model = model
        self.show = show
    }

    public var items: [Item] {
        // A menu bar app has no standard app menu, so "About" lives here, first.
        var out: [Item] = [.about, .divider, .window, .review, .advanced, .divider, .export, .import, .showInFinder]
        if model.canLaunchAtLogin { out += [.divider, .launchAtLogin(on: model.launchAtLogin)] }
        return out + [.divider, .quit]
    }

    public func perform(_ item: Item) {
        switch item {
        case .about:
            NSApp?.activate()
            NSApp?.orderFrontStandardAboutPanel(options: [
                .version: AboutInfo.version(info: Bundle.main.infoDictionary ?? [:]),
                .credits: AboutInfo.credits(),
            ])
        case .window: show(.controls)
        case .review: show(.review)
        case .advanced: show(.settings)
        case .export: exportPanel()
        case .import: importPanel()
        case .showInFinder: NSWorkspace.shared.open(model.snapshotFolder)
        case .launchAtLogin(let on): model.setLaunchAtLogin(!on)
        case .quit:
            model.moduleSettings.flush()  // a slider moved just before quitting is still saved
            NSApplication.shared.terminate(nil)
        case .divider: break
        }
    }

    private func exportPanel() {
        let panel = NSSavePanel()
        panel.title = "Export hEARo snapshots"
        let day = Date().formatted(.iso8601.year().month().day())
        panel.nameFieldStringValue = "hearo-snapshots-\(day).zip"
        panel.allowedContentTypes = [.zip]
        NSApp.activate(ignoringOtherApps: true)
        if panel.runModal() == .OK, let url = panel.url { model.exportSnapshots(to: url) }
    }

    private func importPanel() {
        let panel = NSOpenPanel()
        panel.title = "Import hEARo snapshots"
        panel.allowedContentTypes = [.zip]
        panel.allowsMultipleSelection = false
        NSApp.activate(ignoringOtherApps: true)
        if panel.runModal() == .OK, let url = panel.url { model.importSnapshots(from: url) }
    }

    /// SwiftUI menu content (the popover's ⋯ menu).
    @ViewBuilder public var swiftUIContent: some View {
        ForEach(Array(items.enumerated()), id: \.offset) { _, item in
            switch item {
            case .divider: Divider()
            case .launchAtLogin(let on):
                Toggle(item.title, isOn: Binding(get: { on }, set: { _ in perform(item) }))
            case .quit: Button(item.title) { perform(item) }.keyboardShortcut("q")
            default: Button(item.title) { perform(item) }
            }
        }
    }

    /// AppKit menu (right-click on the menu bar icon).
    public func nsMenu() -> NSMenu {
        let menu = NSMenu()
        for item in items {
            if item == .divider {
                menu.addItem(.separator())
                continue
            }
            let mi = ClosureMenuItem(title: item.title, keyEquivalent: item == .quit ? "q" : "") { perform(item) }
            if case .launchAtLogin(let on) = item { mi.state = on ? .on : .off }
            menu.addItem(mi)
        }
        return menu
    }
}

/// What the About panel shows besides name, icon, version and copyright (from Info.plist).
public enum AboutInfo {
    /// Where the docs live, once the project has a public home; until then the credits say where.
    static let docsURL: URL? = nil

    /// The build part of "Version 0.1.0 (…)": build number and git hash, plus "debug" for
    /// `just app` builds (one app name for every build; the configuration only shows here).
    public static func version(info: [String: Any]) -> String {
        guard let build = info["CFBundleVersion"] as? String else { return "dev" }
        var parts = [build]
        if let sha = info["HearoGitSHA"] as? String, sha != "unknown" { parts.append(sha) }
        if info["HearoBuildConfiguration"] as? String == "debug" { parts.append("debug") }
        return parts.joined(separator: " · ")
    }

    public static func credits() -> NSAttributedString {
        let body = """
            Removes mouth clicks, smacks, harsh sibilance, hum and plosives from the sound your \
            Mac plays, in real time. Audio is processed on this Mac and never sent anywhere; \
            nothing is kept except snapshots you save yourself.

            Free and open source under MIT OR Apache-2.0.
            """
        let docs = docsURL.map { "\n\($0.absoluteString)" } ?? "\nDocumentation: README and docs/user/ in the hEARo source."
        let text = NSMutableAttributedString(
            string: body + docs,
            attributes: [
                .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                .foregroundColor: NSColor.secondaryLabelColor,
            ])
        if let url = docsURL {
            let start = (body as NSString).length + 1
            text.addAttribute(.link, value: url, range: NSRange(location: start, length: (url.absoluteString as NSString).length))
        }
        let centred = NSMutableParagraphStyle()
        centred.alignment = .center
        text.addAttribute(.paragraphStyle, value: centred, range: NSRange(location: 0, length: text.length))
        return text
    }
}

/// NSMenuItem that runs a closure.
final class ClosureMenuItem: NSMenuItem {
    private let run: () -> Void

    init(title: String, keyEquivalent: String, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(fire), keyEquivalent: keyEquivalent)
        target = self
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("not used") }

    @objc private func fire() { run() }
}

/// Shows the app menu when hEARo's menu bar icon is right-clicked (or control-clicked).
/// `MenuBarExtra` has no API for this; a local event monitor sees clicks on the app's own status
/// bar window only, so other apps' icons are never affected.
@MainActor
public final class StatusItemRightClick {
    private var monitor: Any?
    private let menu: () -> NSMenu

    public init(menu: @escaping () -> NSMenu) { self.menu = menu }

    public func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.rightMouseDown, .leftMouseDown]) { [weak self] event in
            guard let self, Self.isStatusBarClick(event), event.type == .rightMouseDown || event.modifierFlags.contains(.control),
                let view = event.window?.contentView
            else { return event }
            NSMenu.popUpContextMenu(self.menu(), with: event, for: view)
            return nil
        }
    }

    static func isStatusBarClick(_ event: NSEvent) -> Bool {
        guard let window = event.window else { return false }
        return String(describing: type(of: window)).contains("StatusBarWindow")
    }

    isolated deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }
}

/// The menu bar icon. Also installs the right-click menu (it needs the scene's `openWindow`).
public struct MenuBarLabel: View {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    @State private var rightClick: StatusItemRightClick?

    public init(model: AppModel) { self.model = model }

    public var body: some View {
        Image(systemName: model.menuBarSymbol)
            .symbolRenderingMode(model.iconState.isAccent ? .palette : .monochrome)
            .foregroundStyle(model.iconState.isAccent ? Color.accentColor : Color.primary)
            .accessibilityLabel("hEARo")
            .onAppear {
                let open = openWindow
                let handler = StatusItemRightClick { [model] in
                    AppMenu(model: model, show: { model.openMainWindow($0, using: open) }).nsMenu()
                }
                handler.install()
                rightClick = handler
            }
    }
}
