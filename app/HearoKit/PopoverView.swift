import SwiftUI

/// Scene id of the snapshot review window (`Window(..., id:)` in the app).
public let reviewWindowID = "review"

/// The menu bar popover (§13.3). M0 shell: header, master switch, modules, footer.
public struct PopoverView: View {
    @Bindable var model: AppModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(model: AppModel) { self.model = model }

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
        .onAppear { model.setPopoverVisible(true, reduceMotion: reduceMotion) }
        .onChange(of: reduceMotion) { _, on in model.setPopoverVisible(true, reduceMotion: on) }
        .onDisappear { model.setPopoverVisible(false) }
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
                Button("Advanced…") { openAdvanced() }
                    .accessibilityLabel("Advanced settings")
                    .buttonStyle(.link)
                    .font(.caption)
                    .help("Every parameter of every module (also in the ⋯ menu)")
            }
        }
    }

    private func openAdvanced() {
        openWindow(id: settingsWindowID)
        NSApp?.activate()
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

extension PopoverView {
    /// ⋯ menu: the same entries as the right-click menu on the menu bar icon (incl. Quit, ⌘Q).
    var moreMenu: some View {
        Menu {
            AppMenu(
                model: model, openReview: { openWindow(id: reviewWindowID) },
                openSettings: { openWindow(id: settingsWindowID) }
            ).swiftUIContent
        } label: {
            Image(systemName: "ellipsis.circle")
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .accessibilityLabel("More")
        .help("Review, export/import snapshots, quit (also on right-click of the menu bar icon)")
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
struct PopoverView_Previews: PreviewProvider {
    static var previews: some View {
        PopoverView(model: AppModel(engine: FakeEngine(.previewRunning)))
            .previewDisplayName("Running")
        PopoverView(model: AppModel(engine: FakeEngine(.previewDenied)))
            .previewDisplayName("Permission denied")
    }
}
