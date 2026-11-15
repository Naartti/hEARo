import SwiftUI

/// The controls (§13.3): header, master switch, live waveform, modules, footer. Shown in the
/// menu bar popover and, to keep it open while testing, in the hEARo window's Controls tab.
public struct ControlsView: View {
    @Bindable var model: AppModel
    /// Where it is shown: the popover (closes on any click elsewhere) or the window.
    let viewer: LiveViewer
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// False while the hEARo window is minimised, hidden or covered (no polling then).
    @Environment(\.liveViewOnScreen) private var onScreen

    public init(model: AppModel, viewer: LiveViewer = .popover) {
        self.model = model
        self.viewer = viewer
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            header
            WaveformView(buffer: model.waveform, names: model.moduleNames)
                .frame(height: 56)
                .opacity(model.isOn ? 1 : 0.4)
            Divider()
            if model.snapshot.permission == .denied {
                PermissionDeniedView()
            } else {
                modules
            }
            if let info = model.infoMessage {
                Label(info, systemImage: "checkmark.circle")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            if let error = model.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Divider()
            labeling
            Divider()
            footer
        }
        .padding(14)
        .frame(width: 360)
        .onAppear { model.setLiveViewVisible(onScreen, viewer: viewer, reduceMotion: reduceMotion) }
        .onChange(of: reduceMotion) { _, on in model.setLiveViewVisible(onScreen, viewer: viewer, reduceMotion: on) }
        .onChange(of: onScreen) { _, on in model.setLiveViewVisible(on, viewer: viewer, reduceMotion: reduceMotion) }
        .onDisappear { model.setLiveViewVisible(false, viewer: viewer) }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Image(systemName: model.menuBarSymbol)
                .foregroundStyle(model.iconState.isAccent ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("hEARo").font(.headline)
                Text(model.statusLine)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if let output = model.outputLine {
                    Text(output)
                        .font(.caption)
                        .foregroundStyle(model.snapshot.problem.isEmpty ? AnyShapeStyle(.secondary) : AnyShapeStyle(.orange))
                        .lineLimit(2)
                        .help("hEARo follows the output chosen in Control Center or System Settings → Sound")
                }
            }
            Spacer()
            if viewer == .popover {
                Button {
                    show(.controls)
                } label: {
                    Image(systemName: "macwindow")
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Open in a window")
                .help("Open these controls in the hEARo window, which stays open")
            }
            Toggle("Processing", isOn: Binding(get: { model.isOn }, set: { model.setOn($0) }))
                .toggleStyle(.switch)
                .labelsHidden()
                .accessibilityLabel("Processing on or off")
        }
    }

    private var labeling: some View {
        HStack {
            Text("§ = \"missed it\" snapshot")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Text("\(model.snapshot.snapshotsSaved) saved")
                .font(.caption)
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    private var modules: some View {
        VStack(alignment: .leading, spacing: 6) {
            if model.snapshot.modules.isEmpty {
                Text("No cleanup modules yet — audio passes through untouched.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ForEach(model.snapshot.modules) { m in
                VStack(alignment: .leading, spacing: 1) {
                    HStack {
                        if m.id == "limiter" {
                            // Ear protection is not optional (HANDOVER §7.5).
                            Label("Safety limiter (always on)", systemImage: "shield.lefthalf.filled")
                                .foregroundStyle(.secondary)
                        } else {
                            // The swatch makes the list the waveform's colour legend.
                            ModuleSwatch(m.id)
                            Toggle(m.name, isOn: Binding(get: { m.enabled }, set: { model.setModule(m.id, enabled: $0) }))
                                .toggleStyle(.checkbox)
                                .accessibilityHint("Marked in \(ModuleStyle.style(m.id).colorName) in the waveform")
                        }
                        Spacer()
                        Text("\(m.triggers)")
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("\(m.triggers) removed this session")
                    }
                    if let info = model.moduleSettings.module(m.id), let p = info.primaryParam {
                        PrimaryParamRow(
                            module: info, param: p, value: model.moduleSettings.value(info, p), enabled: m.enabled,
                            locale: model.locale, onChange: { model.setParam(m.id, p.id, $0) }
                        )
                        .padding(.leading, 18)
                    }
                }
            }
            HStack {
                Spacer()
                // Same name as the ⋯ menu entry, the tab and the app menu's Settings… (DESIGN.md).
                Button(AppMenu.Item.settings.title) { show(.settings) }
                    .buttonStyle(.link)
                    .font(.caption)
                    .help("Every parameter of every module (also in the ⋯ menu)")
            }
        }
    }

    /// In the window, switch tabs; from the popover, open the window on that tab.
    private func show(_ tab: MainTab) {
        if viewer == .window { model.show(tab) } else { model.openMainWindow(tab, using: openWindow) }
    }

    private var footer: some View {
        HStack {
            Toggle("Bypass (A/B)", isOn: Binding(get: { model.snapshot.bypassed }, set: { model.setBypass($0) }))
                .toggleStyle(.button)
                .disabled(!model.isOn)
                .help("Hear the unprocessed audio")
            Spacer()
            moreMenu
        }
    }
}

extension ControlsView {
    /// ⋯ menu: the same entries as the right-click menu on the menu bar icon (incl. Quit, ⌘Q).
    var moreMenu: some View {
        Menu {
            AppMenu(model: model, inWindow: viewer == .window, show: show).swiftUIContent
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .accessibilityLabel("More")
        .help("Window, review, export/import snapshots, quit (also on right-click of the menu bar icon)")
    }
}

/// Shown when System Audio Recording permission is missing (§13.3 special states).
struct PermissionDeniedView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("hEARo can't hear your Mac yet", systemImage: "exclamationmark.triangle")
                .font(.subheadline.weight(.semibold))
            Text("Allow System Audio Recording for hEARo in System Settings → Privacy & Security. Your audio stays on this Mac.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Button("Open System Settings…") {
                if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture") {
                    NSWorkspace.shared.open(url)
                }
            }
        }
    }
}

// PreviewProvider rather than #Preview: the #Preview macro plugin ships only with Xcode, and this
// file must also compile with Command Line Tools (`swift build`). Xcode still shows these.
struct ControlsView_Previews: PreviewProvider {
    static var previews: some View {
        ControlsView(model: AppModel(engine: FakeEngine(.previewRunning)))
            .previewDisplayName("Running")
        ControlsView(model: AppModel(engine: FakeEngine(.previewDenied)))
            .previewDisplayName("Permission denied")
    }
}
