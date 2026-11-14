import AppKit

/// hEARo is a regular app (Dock icon, ⌘Tab, menu bar) exactly while its window is open, and
/// menu-bar-only (`.accessory`, LSUIElement) otherwise — DESIGN.md *Dock icon while the window
/// is open*. The window drives it (appear/close), so every route into the window is covered.
@MainActor
public final class DockPresence {
    /// What DockPresence does to the app; injected so tests can log it instead.
    struct Platform {
        var setPolicy: (NSApplication.ActivationPolicy) -> Void
        /// Hand focus back to the app the user was in before hEARo (if hEARo is still active).
        var restoreFocus: () -> Void
        /// Run after the current event (the window close) has finished.
        var later: (@escaping @MainActor () -> Void) -> Void
    }

    private let platform: Platform
    /// The hEARo window is on screen (or minimised): the menu commands just switch tabs.
    public private(set) var isWindowOpen = false

    init(platform: Platform) { self.platform = platform }

    /// The real app: AppKit activation policy, focus back to the previously active app.
    public convenience init() {
        let previous = PreviousApp()
        self.init(
            platform: Platform(
                setPolicy: { _ = NSApp?.setActivationPolicy($0) },
                restoreFocus: { if NSApp?.isActive == true { previous.app?.activate(from: .current, options: []) } },
                later: { work in DispatchQueue.main.async { MainActor.assumeIsolated { work() } } }))
    }

    /// About to open the window: become regular *before* activation, or the menu bar can fail
    /// to appear.
    func willOpenWindow() { platform.setPolicy(.regular) }

    /// The window appeared (any route, including state restoration).
    public func windowAppeared() {
        isWindowOpen = true
        platform.setPolicy(.regular)
    }

    /// The window is closing: leave the Dock once the close has finished, unless it reopened.
    public func windowClosed() {
        isWindowOpen = false
        platform.later { [self] in
            guard !isWindowOpen else { return }
            platform.setPolicy(.accessory)
            platform.restoreFocus()
        }
    }
}

/// Remembers the last app other than hEARo to become active, so closing the window can give
/// focus back to it (an accessory app left active with no window would keep the menu bar).
@MainActor
private final class PreviousApp {
    private(set) var app: NSRunningApplication?
    private var token: NSObjectProtocol?

    init() {
        let own = ProcessInfo.processInfo.processIdentifier
        let front = NSWorkspace.shared.frontmostApplication
        app = front?.processIdentifier == own ? nil : front
        token = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { [weak self] note in
            let activated = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            MainActor.assumeIsolated {
                guard let activated, activated.processIdentifier != own else { return }
                self?.app = activated
            }
        }
    }

    isolated deinit {
        if let token { NSWorkspace.shared.notificationCenter.removeObserver(token) }
    }
}
