import AppKit
import SwiftUI

// Module parameter controls, shared by the Settings tab of the hEARo window (live settings) and the
// review window's inspector (a trial copy rendered offline). They take values and closures, not
// a model, so both can use them.

private struct CompactParamControlsKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Parameter sliders put their name above the slider (narrow panels).
    public var compactParamControls: Bool {
        get { self[CompactParamControlsKey.self] }
        set { self[CompactParamControlsKey.self] = newValue }
    }
}

/// One parameter: a picker for discrete choices, a stepper for whole-number steps, otherwise a
/// slider with the formatted value beside it.
public struct ParamControl: View {
    let module: ModuleInfo
    let param: ParamInfo
    let value: Float
    let locale: Locale
    let onChange: (Float) -> Void

    public init(module: ModuleInfo, param: ParamInfo, value: Float, locale: Locale = .current, onChange: @escaping (Float) -> Void) {
        self.module = module
        self.param = param
        self.value = value
        self.locale = locale
        self.onChange = onChange
    }

    @Environment(\.compactParamControls) private var compact

    private var text: String { ParamFormat.string(value, param, locale: locale) }
    private var a11yLabel: String { "\(module.name) \(param.name.lowercased())" }

    public var body: some View {
        if compact && param.choices.isEmpty && param.step < 1 {
            // Narrow panels (the review inspector): the name above a full-width slider.
            VStack(alignment: .leading, spacing: 2) {
                Text(param.name).accessibilityHidden(true)
                control.frame(maxWidth: .infinity)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            LabeledContent {
                control
            } label: {
                Text(param.name)
            }
        }
    }

    @ViewBuilder private var control: some View {
        if !param.choices.isEmpty {
            Picker(param.name, selection: Binding(get: { snappedChoice }, set: { onChange($0) })) {
                ForEach(param.choices, id: \.value) { c in Text(c.label).tag(c.value) }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .fixedSize()
            .accessibilityLabel(a11yLabel)
        } else if param.step >= 1 {
            Stepper(value: Binding(get: { Double(value) }, set: { onChange(Float($0)) }), in: range, step: Double(param.step)) {
                Text(text).monospacedDigit()
            }
            .accessibilityLabel(a11yLabel)
            .accessibilityValue(text)
        } else {
            HStack(spacing: 8) {
                Slider(value: Binding(get: { Double(value) }, set: { onChange(Float($0)) }), in: range)
                    .frame(minWidth: compact ? 100 : 140, maxWidth: compact ? .infinity : nil)
                    .accessibilityLabel(a11yLabel)
                    .accessibilityValue(text)
                Text(text)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .frame(width: 64, alignment: .trailing)
                    .accessibilityHidden(true)
            }
        }
    }

    private var range: ClosedRange<Double> { Double(param.min)...Double(max(param.max, param.min)) }
    private var snappedChoice: Float {
        param.choices.min(by: { abs($0.value - value) < abs($1.value - value) })?.value ?? value
    }
}

/// One module's section: swatch, name, on/off, every parameter, "Reset to defaults".
public struct ModuleSettingsSection: View {
    let module: ModuleInfo
    let settings: EngineSettings
    let locale: Locale
    let onParam: (String, Float) -> Void
    let onEnabled: (Bool) -> Void
    let onReset: () -> Void

    public init(
        module: ModuleInfo, settings: EngineSettings, locale: Locale = .current, onParam: @escaping (String, Float) -> Void,
        onEnabled: @escaping (Bool) -> Void, onReset: @escaping () -> Void
    ) {
        self.module = module
        self.settings = settings
        self.locale = locale
        self.onParam = onParam
        self.onEnabled = onEnabled
        self.onReset = onReset
    }

    @Environment(\.compactParamControls) private var compact
    private var enabled: Bool { module.alwaysOn || settings.isEnabled(module.id) }
    private var isDefault: Bool { module.params.allSatisfy { settings.value(module, $0) == $0.defaultValue } }

    public var body: some View {
        Section {
            ForEach(module.params) { p in
                ParamControl(module: module, param: p, value: settings.value(module, p), locale: locale) { onParam(p.id, $0) }
                    .disabled(!enabled)
            }
        } header: {
            header
        } footer: {
            Text(module.description)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            if module.cleanup { ModuleSwatch(module.id, size: 11) }
            if module.alwaysOn {
                Label("\(module.name) (always on)", systemImage: "shield.lefthalf.filled")
                    .labelStyle(.titleOnly)
                    .font(.headline)
            } else {
                Toggle(isOn: Binding(get: { enabled }, set: { onEnabled($0) })) {
                    Text(module.name).font(.headline)
                }
                .toggleStyle(.checkbox)
                .accessibilityLabel(module.name)
                .accessibilityHint(module.cleanup ? "Marked in \(ModuleStyle.style(module.id).colorName) in the waveform" : "")
            }
            Spacer()
            Button(compact ? "Reset" : "Reset to defaults", action: onReset)
                .controlSize(.small)
                .disabled(isDefault)
                .accessibilityLabel("Reset \(module.name) to defaults")
        }
    }
}

/// The hEARo window's Settings tab ("Advanced settings"): every parameter of every module, live.
public struct AdvancedSettingsView: View {
    @Bindable var model: AppModel
    @State private var confirmReset = false

    public init(model: AppModel) { self.model = model }

    private var tuning: ModuleSettingsModel { model.moduleSettings }

    public var body: some View {
        VStack(spacing: 0) {
            Form {
                ForEach(tuning.catalog) { m in
                    ModuleSettingsSection(
                        module: m, settings: tuning.settings, locale: model.locale,
                        onParam: { model.setParam(m.id, $0, $1) },
                        onEnabled: { model.setModule(m.id, enabled: $0) },
                        onReset: { tuning.reset(m.id) })
                }
                AutoCaptureSection(model: model)
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                if let error = tuning.saveError {
                    Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                } else {
                    Text(tuning.isPersistent ? "Changes apply immediately and are remembered." : "Changes apply immediately.")
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Reset All…") { confirmReset = true }
                    .confirmationDialog("Reset every module's settings to the defaults?", isPresented: $confirmReset) {
                        Button("Reset All", role: .destructive) { tuning.resetAll() }
                    } message: {
                        Text("Which modules are on stays as it is.")
                    }
            }
            .font(.callout)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        }
        .frame(minWidth: 480, idealWidth: 520, minHeight: 420, idealHeight: 640)
        .onDisappear { tuning.flush() }
    }
}

/// Settings tab: auto-save a clip around every de-smack catch for the dataset (ADR 0018).
struct AutoCaptureSection: View {
    @Bindable var model: AppModel

    private func binding<T>(_ key: WritableKeyPath<AutoCaptureSettings, T>) -> Binding<T> {
        Binding(
            get: { model.autoCapture[keyPath: key] },
            set: {
                var s = model.autoCapture
                s[keyPath: key] = $0
                model.setAutoCapture(s)
            })
    }

    var body: some View {
        let limits = model.autoCaptureLimits
        Section {
            Toggle("Save a clip around every removed smack", isOn: binding(\.enabled))
            LabeledContent("Daily limit") {
                number(binding(\.dailyCap), in: 1...limits.maxDailyCap, step: 10, unit: "clips", label: "Daily limit")
            }
            LabeledContent("Delete unreviewed auto-saved clips after") {
                number(
                    binding(\.retentionDays), in: 1...limits.maxRetentionDays, step: 1, unit: "days",
                    label: "Delete unreviewed auto-saved clips after")
            }
            LabeledContent("Saved in the last 24 hours") {
                Text("\(model.snapshot.autoSavedLast24h) of \(model.autoCapture.dailyCap)").monospacedDigit()
            }
        } header: {
            HStack(spacing: 8) {
                ModuleSwatch("desmack", size: 11)
                Text("Auto-save de-smack catches").font(.headline)
            }
        } footer: {
            Text(
                "Builds the training dataset while you listen. Each clip is 8 s of the original sound with the catch pre-marked; it stays on this Mac and appears as Auto in the Review tab, where A accepts the catch and X marks a false positive. Off by default. Clips you reviewed or edited are never deleted automatically."
            )
            .font(.caption)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func number(
        _ value: Binding<Int>, in range: ClosedRange<Int>, step: Int, unit: String, label: String
    ) -> some View {
        HStack(spacing: 6) {
            TextField(label, value: value, format: .number)
                .labelsHidden()
                .accessibilityLabel("\(label), \(unit)")
                .help("\(range.lowerBound)–\(range.upperBound) \(unit)")
                .multilineTextAlignment(.trailing)
                .frame(width: 64)
            Text(unit).foregroundStyle(.secondary).accessibilityHidden(true)
            Stepper("Change \(label.lowercased())", value: value, in: range, step: step).labelsHidden()
        }
        .accessibilityElement(children: .contain)
    }
}

/// The popover's per-module "amount" slider (the module's primary parameter), under its name.
struct PrimaryParamRow: View {
    let module: ModuleInfo
    let param: ParamInfo
    let value: Float
    let enabled: Bool
    let locale: Locale
    let onChange: (Float) -> Void

    var body: some View {
        let text = ParamFormat.string(value, param, locale: locale)
        HStack(spacing: 6) {
            Text(param.name)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 70, alignment: .leading)
                .accessibilityHidden(true)
            Slider(
                value: Binding(get: { Double(value) }, set: { onChange(Float($0)) }),
                in: Double(param.min)...Double(max(param.max, param.min))
            )
            .controlSize(.mini)
            .accessibilityLabel("\(module.name) \(param.name.lowercased())")
            .accessibilityValue(text)
            Text(text)
                .font(.caption)
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .frame(width: 50, alignment: .trailing)
                .accessibilityHidden(true)
        }
        .disabled(!enabled)
        .help("\(param.name) — \(module.description)")
    }
}

// PreviewProvider rather than #Preview (no macro plugin with Command Line Tools).
struct AdvancedSettingsView_Previews: PreviewProvider {
    static var previews: some View {
        AdvancedSettingsView(model: AppModel(engine: FakeEngine(.previewRunning)))
            .frame(width: 520, height: 700)
    }
}
