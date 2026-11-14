import SwiftUI

/// The review window's trailing processing panel: A/B bypass, render status, live-settings
/// buttons, and every module's toggle and parameters (the Advanced settings controls, compact).
/// Edits apply to a trial copy rendered offline for this clip; the live sound is untouched until
/// "Apply these to live".
struct ReviewInspector: View {
    @Bindable var inspector: ReviewInspectorModel
    let locale: Locale

    var body: some View {
        Form {
            Section {
                Toggle(isOn: Binding(get: { inspector.bypass }, set: { inspector.setBypass($0) })) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Bypass")
                        Text(inspector.bypass ? "Hearing the original" : "Hearing the processed clip")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .toggleStyle(.switch)
                .help("B — switch between the original and the processed clip at the same position")
                .accessibilityLabel("Bypass")
                .accessibilityValue(inspector.bypass ? "On, hearing the original" : "Off, hearing the processed clip")
                .accessibilityHint("Press B to switch")
                status
                HStack {
                    Button("Use my live settings") { inspector.useLiveSettings() }
                        .disabled(inspector.matchesLive)
                        .help("Start over from the settings hEARo uses right now")
                    Spacer()
                    Button("Apply these to live") { inspector.applyToLive() }
                        .disabled(inspector.matchesLive)
                        .help("Use these settings for everything hEARo processes (and remember them)")
                }
                .controlSize(.small)
            } header: {
                Text("Processing")
            } footer: {
                Text("Changes here re-render this clip only. Coloured bands show what each module turned down.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(inspector.catalog) { m in
                ModuleSettingsSection(
                    module: m, settings: inspector.settings, locale: locale,
                    onParam: { inspector.set(m.id, $0, $1) },
                    onEnabled: { inspector.setEnabled(m.id, $0) },
                    onReset: { inspector.reset(m.id) })
            }
        }
        .formStyle(.grouped)
        .environment(\.compactParamControls, true)
        .controlSize(.small)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Processing inspector")
    }

    @ViewBuilder private var status: some View {
        HStack(spacing: 6) {
            switch inspector.state {
            case .idle:
                Label("No clip open", systemImage: "waveform").foregroundStyle(.secondary)
            case .rendering:
                ProgressView().controlSize(.mini)
                Text("Rendering…").foregroundStyle(.secondary)
            case .ready(let ms):
                Label(readyText(ms), systemImage: "checkmark.circle").foregroundStyle(.secondary)
            case .failed(let message):
                Label(message, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
                    .lineLimit(3)
            }
        }
        .font(.caption)
        .accessibilityElement(children: .combine)
    }

    private func readyText(_ ms: Double) -> String {
        let events = inspector.result?.events.count ?? 0
        let msText = ms.formatted(.number.precision(.fractionLength(0)).locale(locale))
        return "Rendered in \(msText) ms · \(events) event\(events == 1 ? "" : "s")"
    }
}

// PreviewProvider rather than #Preview (no macro plugin with Command Line Tools).
struct ReviewInspector_Previews: PreviewProvider {
    static var previews: some View {
        if let inspector = ReviewModel.preview(live: AppModel(engine: FakeEngine(.previewRunning))).inspector {
            ReviewInspector(inspector: inspector, locale: .current)
                .frame(width: 320, height: 700)
        }
    }
}
