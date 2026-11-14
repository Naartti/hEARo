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

    public var systemImage: String {
        switch self {
        case .controls: "slider.horizontal.3"
        case .review: "waveform.badge.magnifyingglass"
        case .settings: "gearshape"
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
    /// Open (or bring forward) the hEARo window on `tab`. hEARo is menu-bar-only (LSUIElement),
    /// so the app is activated too, or the window would open behind the frontmost app.
    public func openMainWindow(_ tab: MainTab, using openWindow: OpenWindowAction) {
        show(tab)
        openWindow(id: mainWindowID)
        NSApp?.activate()
    }
}

/// The one hEARo window: Controls · Review · Settings as a segmented control in the toolbar
/// (the Activity Monitor pattern). It stays open until closed, unlike the popover.
public struct MainWindowView: View {
    @Bindable var model: AppModel
    let review: ReviewModel

    public init(model: AppModel, review: ReviewModel) {
        self.model = model
        self.review = review
    }

    public var body: some View {
        content
            .navigationTitle("hEARo")
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
                    .help("Controls ⌘1 · Review ⌘2 · Settings ⌘3")
                }
            }
            .background {
                // ⌘1–⌘3 switch tabs (an LSUIElement app has no View menu to carry them).
                ForEach(MainTab.allCases) { tab in
                    Button(tab.title) { model.show(tab) }
                        .keyboardShortcut(KeyEquivalent(tab.shortcut), modifiers: .command)
                        .opacity(0)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
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
}
