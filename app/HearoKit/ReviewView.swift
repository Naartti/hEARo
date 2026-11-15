import AppKit
import SwiftUI

/// The snapshot review & annotation view (M6), the hEARo window's Review tab: clip list on the left, the timeline editor
/// (waveform + optional spectrogram, `ReviewTimeline.swift`) in the middle, the processing
/// inspector (`ReviewInspector.swift`) on the right. Keyboard: Space play/pause (clip under the
/// mouse), 1–6 region class, ⌫ delete region, ⌘⌫ move clip to Trash, ↑/↓ previous/next clip,
/// Return mark reviewed + next, Esc deselect, ⇧⌘S spectrogram, B bypass (A/B), ⌥⌘I inspector,
/// A accept / X false positive for the shown (else next undecided) detector proposal.
public struct ReviewView: View {
    @Bindable var model: ReviewModel
    /// Number formatting in the inspector.
    let locale: Locale
    @FocusState private var focused: Bool
    @AppStorage("review.showSpectrogram") private var showSpectrogram = true
    @AppStorage("review.spectrogramResolution") private var resolution = SpectrogramResolution.balanced
    @AppStorage("review.showInspector") private var showInspector = true
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(model: ReviewModel, locale: Locale = .current) {
        self.model = model
        self.locale = locale
    }

    private var inspectorShown: Bool { showInspector && model.inspector != nil }

    public var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                ClipList(model: model)
                    .frame(width: 250)
                Divider()
                detail
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                if inspectorShown, let inspector = model.inspector {
                    Divider()
                    ReviewInspector(inspector: inspector, locale: locale)
                        .frame(width: 320)
                        .transition(.move(edge: .trailing).combined(with: .opacity))
                }
            }
            Divider()
            footer
        }
        .frame(minWidth: inspectorShown ? 1100 : 820, minHeight: 460)
        .background {
            // ⇧⌘S / ⌥⌘I work whether or not a clip is open (the header toggles only exist with one).
            Button("Toggle spectrogram") { spectrogramToggle.wrappedValue.toggle() }
                .keyboardShortcut("s", modifiers: [.command, .shift])
                .opacity(0)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            Button("Toggle processing inspector") { inspectorToggle.wrappedValue.toggle() }
                .keyboardShortcut("i", modifiers: [.command, .option])
                .opacity(0)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        // Space, arrows, Return and ⌫ belong to a focused inspector control (slider, checkbox…)
        // when one has focus; the window gets them only while it has focus itself.
        .onKeyPress(.space) {
            guard focused else { return .ignored }
            model.togglePlay()
            return .handled
        }
        .onKeyPress(.upArrow) {
            guard focused else { return .ignored }
            model.selectPrevious()
            return .handled
        }
        .onKeyPress(.downArrow) {
            guard focused else { return .ignored }
            model.selectNext()
            return .handled
        }
        .onKeyPress(.return) {
            guard focused else { return .ignored }
            model.markReviewedAndAdvance()
            return .handled
        }
        .onKeyPress(keys: [.delete, .deleteForward]) { press in
            guard focused else { return .ignored }
            if press.modifiers.contains(.command) {
                model.deleteCurrentClip()
            } else {
                model.deleteSelectedRegion()
            }
            return .handled
        }
        .onKeyPress(.escape) {
            model.selectedRegion = nil
            return .handled
        }
        .onKeyPress(characters: .decimalDigits, phases: .down) { press in
            // ⌘1–⌘3 switch the window's tabs; only bare digits pick a class.
            guard press.modifiers.isDisjoint(with: [.command, .option, .control]), let d = Int(press.characters),
                let cls = RegionClass(key: d)
            else { return .ignored }
            model.setClass(cls)
            return .handled
        }
        // A / X: verify a detector proposal (accept = it is a smack) or mark it a false positive.
        .onKeyPress(characters: CharacterSet(charactersIn: "aAxX"), phases: .down) { press in
            guard focused, press.modifiers.isDisjoint(with: [.command, .option, .control]) else { return .ignored }
            if press.characters.lowercased() == "a" { model.acceptNextProposal() } else { model.rejectNextProposal() }
            return .handled
        }
        .onKeyPress(characters: CharacterSet(charactersIn: "bB"), phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .option, .control]), let inspector = model.inspector else {
                return .ignored
            }
            inspector.toggleBypass()
            return .handled
        }
        .onAppear {
            model.inspector?.windowAppeared()
            model.reload()
            focused = true
        }
        .onDisappear {
            // Another tab (or the window closed): stop playback; the window drops the caches.
            model.tabHidden()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.reload()
        }
    }

    @ViewBuilder private var detail: some View {
        if let info = model.current, let annotation = model.annotation, let audio = model.audio {
            VStack(alignment: .leading, spacing: 10) {
                DetailHeader(
                    model: model, info: info, annotation: annotation, showSpectrogram: spectrogramToggle,
                    showInspector: inspectorToggle)
                TimelineEditor(
                    model: model, audio: audio, annotation: annotation, showSpectrogram: showSpectrogram, resolution: resolution,
                    refocus: { focused = true }
                )
                .frame(minHeight: showSpectrogram ? 260 : 200)
                SelectionBar(model: model, annotation: annotation, showSpectrogram: showSpectrogram, resolution: $resolution)
                ProposalList(model: model, annotation: annotation, refocus: { focused = true })
            }
            .padding(14)
        } else if let error = model.errorMessage {
            ContentUnavailableView("Couldn't open the snapshot", systemImage: "exclamationmark.triangle", description: Text(error))
        } else {
            ContentUnavailableView(
                "No snapshots yet", systemImage: "ear",
                description: Text("Press § when hEARo misses a sound; the last 8 seconds are saved here for labeling."))
        }
    }

    /// Show/hide animates the lane unless Reduce Motion is on.
    private var spectrogramToggle: Binding<Bool> {
        Binding(
            get: { showSpectrogram },
            set: { on in withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.2)) { showSpectrogram = on } })
    }

    /// Show/hide slides the inspector in unless Reduce Motion is on.
    private var inspectorToggle: Binding<Bool> {
        Binding(
            get: { showInspector },
            set: { on in withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.2)) { showInspector = on } })
    }

    private var footer: some View {
        HStack {
            Label("Clips may contain other people's voices and never leave this Mac unless you export them.", systemImage: "lock")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            if let error = model.errorMessage, model.annotation != nil {
                Label(error, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
            }
            Text("Space play · drag to mark · 1–6 class · ⌫ region · ⌘⌫ trash clip · ↑↓ clip · ⏎ done · ⇧⌘S spectrogram · B bypass · ⌥⌘I inspector")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .lineLimit(1)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }
}

// MARK: - List

struct ClipList: View {
    @Bindable var model: ReviewModel

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 2) {
                    ForEach(Array(model.clips.enumerated()), id: \.element.id) { i, clip in
                        ClipRow(clip: clip, selected: i == model.selectedIndex)
                            .id(clip.id)
                            .contentShape(Rectangle())
                            .onTapGesture { model.select(i) }
                            .contextMenu {
                                Button("Move to Trash", systemImage: "trash", role: .destructive) { model.deleteClip(at: i) }
                            }
                            .onHover { inside in
                                if inside {
                                    model.hoveredRow = i
                                } else if model.hoveredRow == i {
                                    model.hoveredRow = nil
                                }
                            }
                    }
                }
                .padding(6)
            }
            .onChange(of: model.selectedIndex) { _, i in
                guard let i, model.clips.indices.contains(i) else { return }
                withAnimation(.easeOut(duration: 0.15)) { proxy.scrollTo(model.clips[i].id) }
            }
        }
        .overlay {
            if model.clips.isEmpty { Text("No snapshots").foregroundStyle(.secondary) }
        }
    }
}

struct ClipRow: View {
    let clip: SnapshotInfo
    let selected: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: clip.reviewed ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(clip.reviewed ? AnyShapeStyle(.green) : AnyShapeStyle(.tertiary))
                .accessibilityLabel(clip.reviewed ? "Reviewed" : "Not reviewed")
            VStack(alignment: .leading, spacing: 2) {
                Text(clip.date.formatted(date: .abbreviated, time: .standard))
                    .font(.callout)
                    .monospacedDigit()
                HStack(spacing: 6) {
                    Text(clip.labelTitle)
                    if clip.regionCount > 0 { Text("\(clip.regionCount) marked") }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            if clip.proposalCount > 0 {
                Image(systemName: "sparkle.magnifyingglass")
                    .foregroundStyle(.secondary)
                    .help("\(clip.proposalCount) detector proposals")
                    .accessibilityLabel("\(clip.proposalCount) proposals")
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(selected ? AnyShapeStyle(.selection) : AnyShapeStyle(.clear), in: RoundedRectangle(cornerRadius: 6))
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

// MARK: - Detail pieces

struct DetailHeader: View {
    @Bindable var model: ReviewModel
    let info: SnapshotInfo
    let annotation: Annotation
    @Binding var showSpectrogram: Bool
    @Binding var showInspector: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(info.date.formatted(date: .long, time: .standard)).font(.headline)
                Text("\(info.labelTitle) · \(String(format: "%.1f", annotation.duration)) s · \(Int(annotation.sampleRate)) Hz")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button {
                model.togglePlay()
            } label: {
                Label(model.player?.isPlaying == true ? "Pause" : "Play", systemImage: model.player?.isPlaying == true ? "pause.fill" : "play.fill")
            }
            .help("Space")
            ControlGroup {
                Button("Zoom out", systemImage: "minus.magnifyingglass") {
                    model.viewport.zoom(by: 0.5, around: model.viewport.start + model.viewport.length / 2)
                }
                Button("Zoom to hint", systemImage: "scope") { model.viewport.show(annotation.hint, padding: 0.1) }
                Button("Show all", systemImage: "arrow.left.and.right") { model.viewport.reset() }
                Button("Zoom in", systemImage: "plus.magnifyingglass") {
                    model.viewport.zoom(by: 2, around: model.viewport.start + model.viewport.length / 2)
                }
            }
            .fixedSize()
            Toggle(isOn: $showSpectrogram) {
                Label("Spectrogram", systemImage: "rectangle.split.1x2")
            }
            .toggleStyle(.button)
            .labelStyle(.iconOnly)
            .help("Show the spectrogram under the waveform (⇧⌘S)")
            VStack(spacing: 2) {
                Toggle(isOn: Binding(get: { annotation.reviewed }, set: { model.setReviewed($0) })) {
                    Label("Reviewed", systemImage: "checkmark.circle")
                }
                .toggleStyle(.button)
                .help(ReviewedHint.explanation)
                .accessibilityHint(ReviewedHint.explanation)
                // The caption repeats the hint for sighted users; VoiceOver hears the full one.
                Text(ReviewedHint.caption(reviewed: annotation.reviewed))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .fixedSize()
                    .accessibilityHidden(true)
            }
            Button("Move to Trash", systemImage: "trash", role: .destructive) { model.deleteCurrentClip() }
                .labelStyle(.iconOnly)
                .help("Remove this clip from the dataset (⌘⌫). It goes to the Trash, and importing won't bring it back.")
            if model.inspector != nil {
                Toggle(isOn: $showInspector) {
                    Label("Processing inspector", systemImage: "sidebar.trailing")
                }
                .toggleStyle(.button)
                .labelStyle(.iconOnly)
                .help("Module settings for this clip, and bypass for A/B (⌥⌘I)")
            }
        }
    }
}

/// What marking a clip reviewed promises (ADR 0017, docs/ml/README.md): every mouth noise in it
/// is marked, so all unmarked audio is used as negative training data. Shown next to the header's
/// Reviewed toggle (caption), as its tooltip and as its VoiceOver hint.
enum ReviewedHint {
    static func caption(reviewed: Bool) -> String {
        reviewed ? "Every mouth noise marked" : "Mark every mouth noise first"
    }

    static let explanation =
        "Reviewed means every mouth noise in this clip is marked: unmarked audio is used as clean training data. Return marks reviewed and opens the next clip."
}

struct SelectionBar: View {
    @Bindable var model: ReviewModel
    let annotation: Annotation
    let showSpectrogram: Bool
    @Binding var resolution: SpectrogramResolution

    var body: some View {
        HStack(spacing: 12) {
            if let id = model.selectedRegion, let r = annotation.region(id) {
                Text("\(ms(r.start)) – \(ms(r.end))  (\(ms(r.length)))")
                    .font(.callout.monospacedDigit())
                    .textSelection(.enabled)
                Picker("Class", selection: Binding(get: { r.cls }, set: { model.setClass($0) })) {
                    ForEach(RegionClass.allCases) { c in
                        Text("\(c.key)  \(c.title)").tag(c)
                    }
                }
                .fixedSize()
                Button("Delete", systemImage: "trash", role: .destructive) { model.deleteSelectedRegion() }
                    .help("⌫")
            } else {
                Text("Playhead \(ms(model.playhead)) · \(annotation.regions.count) region\(annotation.regions.count == 1 ? "" : "s")")
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.secondary)
                Text("Drag on the waveform or spectrogram to mark a sound; ⌘-scroll or pinch to zoom.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            Spacer()
            if showSpectrogram {
                Picker("Spectrogram", selection: $resolution) {
                    ForEach(SpectrogramResolution.allCases) { r in
                        Text(r.title).tag(r)
                    }
                }
                .pickerStyle(.menu)
                .fixedSize()
                .accessibilityLabel("Spectrogram resolution")
                .help("Time detail sharpens clicks and smacks; frequency detail separates hum harmonics")
            }
        }
        .frame(minHeight: 24)
    }

    private func ms(_ s: Double) -> String { String(format: "%.2f ms", s * 1000) }
}

struct ProposalList: View {
    @Bindable var model: ReviewModel
    let annotation: Annotation
    /// Give keyboard focus back to the review view (a clicked button takes it, and A / X, Space
    /// and Return only work while the view has it).
    let refocus: () -> Void

    var body: some View {
        if !annotation.proposals.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("Detector proposals").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                let open = annotation.proposals.filter { annotation.decision(for: $0) == .undecided }.count
                Text(
                    open > 0
                        ? "\(open) to verify — A: accept · X: false positive · Return: reviewed & next"
                        : "All verified — Return: reviewed & next"
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(annotation.proposals) { p in
                            row(p)
                        }
                    }
                }
                .frame(maxHeight: 96)
            }
        }
    }

    private func row(_ p: Proposal) -> some View {
        let decision = annotation.decision(for: p)
        let target = model.focusedProposal ?? annotation.firstUndecided
        return HStack(spacing: 8) {
            Image(systemName: target == p.id ? "arrowtriangle.right.fill" : "circle.fill")
                .imageScale(.small)
                .foregroundStyle(target == p.id ? AnyShapeStyle(.primary) : AnyShapeStyle(.clear))
                .accessibilityHidden(true)
            Text(String(format: "%.1f–%.1f ms", p.start * 1000, p.end * 1000)).monospacedDigit()
                .strikethrough(decision == .rejected)
            Text(p.classString.replacingOccurrences(of: "_", with: " "))
            Text(p.module).foregroundStyle(.secondary)
            if let db = p.dbRemoved { Text(String(format: "%.1f dB", db)).foregroundStyle(.secondary) }
            if p.inPressWindow { Image(systemName: "scope").foregroundStyle(.secondary).help("In the press window") }
            Spacer()
            let at = String(format: "%.0f ms", p.start * 1000)
            Button("Show") {
                model.showProposal(p.id)
                refocus()
            }
            .help("Zoom to it; A / X then act on this one")
            .accessibilityLabel("Show proposal at \(at)")
            Button(decision == .accepted ? "Accepted" : "Accept (A)", systemImage: "checkmark") {
                model.accept(proposal: p.id)
                refocus()
            }
            .disabled(decision == .accepted)
            .help("It is what the detector says: keep it as a marked region (A)")
            .accessibilityLabel("Accept proposal at \(at)")
            Button(decision == .rejected ? "False positive" : "False positive (X)", systemImage: "xmark") {
                model.reject(proposal: p.id)
                refocus()
            }
            .disabled(decision == .rejected)
            .help("The detector was wrong: keep it as a hard negative for training (X)")
            .accessibilityLabel("Mark proposal at \(at) as a false positive")
        }
        .labelStyle(.titleAndIcon)
        .font(.caption)
        .controlSize(.small)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(
            "Proposal \(String(format: "%.0f", p.start * 1000)) milliseconds, \(decision == .accepted ? "accepted" : decision == .rejected ? "false positive" : "not verified")"
        )
        .accessibilityValue(target == p.id ? "Next to verify: A accepts, X marks a false positive" : "")
    }
}

// PreviewProvider rather than #Preview (no macro plugin with Command Line Tools).
struct ReviewView_Previews: PreviewProvider {
    static var previews: some View {
        ReviewView(model: .preview(live: AppModel(engine: FakeEngine(.previewRunning))))
            .frame(width: 1000, height: 560)
    }
}
