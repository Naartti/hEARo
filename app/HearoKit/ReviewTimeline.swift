import AppKit
import SwiftUI

// The review window's timeline: stacked lanes (waveform, optional spectrogram) over one shared
// time axis and `Viewport`. `TimelineEditor` owns the gestures (drag to create/move/resize
// regions, click to place the playhead, scroll/pinch to zoom) for the whole stack, so every lane
// edits the same way. Each lane is a plain `Canvas` that draws its content between
// `TimelineMarks.under` and `.over`, so the hint, regions and proposals line up across lanes;
// the playhead is one overlay above all lanes (the only thing redrawn at display rate while
// playing). New lanes or overlay layers (e.g. per-module colour overlays) slot into
// `TimelineEditor.lanes` and receive the same `Viewport`.

extension RegionClass {
    var color: Color {
        switch self {
        case .lipSmack: .red
        case .mouthClick: .orange
        case .plosive: .purple
        case .breath: .teal
        case .filledPause: .blue
        case .other: .gray
        }
    }
}

struct TimelineEditor: View {
    @Bindable var model: ReviewModel
    let audio: AudioClip
    let annotation: Annotation
    let showSpectrogram: Bool
    let resolution: SpectrogramResolution
    let refocus: () -> Void

    private enum Mode {
        case pending(RegionHit, start: Double)
        case create(anchor: Double, id: UUID?)
        case move(UUID, grab: Double)
        case resize(UUID, Annotation.Edge)
    }

    @State private var mode: Mode?
    @State private var lastClick: (date: Date, x: CGFloat)?
    @State private var overEdge = false

    /// Pixels within which an edge can be grabbed.
    private let edgePx = 5.0
    static let axisHeight = 16.0
    static let laneGap = 4.0

    var body: some View {
        GeometryReader { geo in
            let width = Double(geo.size.width)
            let lanesHeight = max(0, Double(geo.size.height) - Self.axisHeight)
            // Spectrogram gets 60 % when shown (it carries more information per pixel).
            let waveHeight = showSpectrogram ? max(60, (lanesHeight - Self.laneGap) * 0.4) : lanesHeight
            VStack(spacing: 0) {
                ZStack(alignment: .top) {
                    lanes(waveHeight: waveHeight, width: width)
                    PlayheadOverlay(model: model)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
                .frame(height: lanesHeight)
                TimeAxis(viewport: model.viewport)
                    .frame(height: Self.axisHeight)
                    .accessibilityHidden(true)
            }
            .background(
                ScrollZoomCatcher(
                    onScroll: { dx, dy, zoom, precise, xFrac in scroll(dx: dx, dy: dy, zoom: zoom, precise: precise, xFrac: xFrac, width: width) },
                    onMagnify: { mag, xFrac in
                        model.viewport.zoom(by: 1 + mag, around: model.viewport.start + xFrac * model.viewport.length)
                    })
            )
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { v in dragChanged(v, width: width) }
                    .onEnded { v in dragEnded(v, width: width) }
            )
            .onContinuousHover { phase in
                switch phase {
                case .active(let p):
                    if model.hoveredRow != nil { model.hoveredRow = nil }
                    let edge: Bool
                    switch annotation.hit(at: time(p.x, width), tolerance: tol(width)) {
                    case .startEdge, .endEdge: edge = true
                    case .inside, .none: edge = false
                    }
                    if edge != overEdge { overEdge = edge }
                case .ended:
                    if overEdge { overEdge = false }
                }
            }
            .pointerStyle(overEdge ? .columnResize : nil)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Clip timeline")
    }

    @ViewBuilder private func lanes(waveHeight: Double, width: Double) -> some View {
        VStack(spacing: Self.laneGap) {
            WaveformLane(model: model, audio: audio, annotation: annotation)
                .frame(height: waveHeight)
            if showSpectrogram {
                SpectrogramLane(model: model, audio: audio, annotation: annotation, resolution: resolution, width: width)
                    .frame(maxHeight: .infinity)
                    .transition(.opacity)
            }
        }
    }

    private func time(_ x: Double, _ width: Double) -> Double { model.viewport.time(atX: x, width: width) }
    private func tol(_ width: Double) -> Double { edgePx / max(width, 1) * model.viewport.length }

    // MARK: Gestures

    private func dragChanged(_ v: DragGesture.Value, width: Double) {
        let t = min(max(0, time(v.location.x, width)), annotation.duration)
        if mode == nil {
            let t0 = time(v.startLocation.x, width)
            mode = .pending(annotation.hit(at: t0, tolerance: tol(width)), start: t0)
        }
        if case .pending(let hit, let t0) = mode {
            guard abs(v.translation.width) >= 3 else { return }  // still a click
            switch hit {
            case .none: mode = .create(anchor: t0, id: nil)
            case .inside(let id): mode = .move(id, grab: t0 - (annotation.region(id)?.start ?? t0))
            case .startEdge(let id): mode = .resize(id, .start)
            case .endEdge(let id): mode = .resize(id, .end)
            }
        }
        switch mode {
        case .create(let anchor, let id):
            if let id {
                model.edit { $0.setBounds(id, anchor, t) }
            } else {
                var new: UUID?
                model.edit { new = $0.addRegion(from: anchor, to: t) }
                if let new {
                    model.selectedRegion = new
                    mode = .create(anchor: anchor, id: new)
                }
            }
        case .move(let id, let grab):
            model.selectedRegion = id
            model.edit { $0.move(id, toStart: t - grab) }
        case .resize(let id, let edge):
            model.selectedRegion = id
            model.edit { $0.resize(id, edge: edge, to: t) }
        case .pending, nil:
            break
        }
    }

    private func dragEnded(_ v: DragGesture.Value, width: Double) {
        defer {
            mode = nil
            refocus()
        }
        guard case .pending(let hit, let t0) = mode else {
            model.commit()
            lastClick = nil
            return
        }
        // A click.
        let now = Date()
        let isDouble = lastClick.map { now.timeIntervalSince($0.date) < NSEvent.doubleClickInterval && abs($0.x - v.location.x) < 4 } ?? false
        lastClick = (now, v.location.x)
        switch hit {
        case .none:
            model.selectedRegion = nil
            model.playhead = min(max(0, t0), annotation.duration)
            model.player?.stop()
        case .inside(let id), .startEdge(let id), .endEdge(let id):
            model.selectedRegion = id
            if isDouble { model.zoom(toRegion: id) }
        }
    }

    private func scroll(dx: Double, dy: Double, zoom: Bool, precise: Bool, xFrac: Double, width: Double) {
        let vp = model.viewport
        if zoom {
            let factor = exp(dy * (precise ? 0.01 : 0.1))
            model.viewport.zoom(by: factor, around: vp.start + xFrac * vp.length)
        } else {
            let d = abs(dx) > abs(dy) ? dx : dy
            let px = precise ? d : d * 20
            model.viewport.pan(by: -px / max(width, 1) * vp.length)
        }
    }

    /// 1-2-5 steps from 1 ms to 10 s.
    static func niceStep(_ raw: Double) -> Double {
        for s in [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5] where s >= raw { return s }
        return 10
    }
}

// MARK: - Shared marks

/// Hint window, press marker, regions and proposals, drawn identically on every lane.
/// `under` goes beneath a lane's content, `over` on top of it. `onDark` lanes (the spectrogram)
/// get outlines and light strokes instead of the light-background tints.
struct TimelineMarks {
    let viewport: Viewport
    let annotation: Annotation
    let selected: UUID?
    let size: CGSize
    var onDark = false

    private var w: Double { Double(size.width) }
    private var h: Double { Double(size.height) }
    private func x(_ t: Double) -> Double { viewport.x(at: t, width: w) }
    private func span(_ a: Double, _ b: Double, inset: Double = 0) -> CGRect {
        let x0 = x(a)
        return CGRect(x: x0, y: inset, width: max(1, x(b) - x0), height: h - 2 * inset)
    }

    func under(_ ctx: inout GraphicsContext, labels: Bool) {
        // Hint window (where the event most likely is).
        let hint = span(annotation.hint.lowerBound, annotation.hint.upperBound)
        if onDark {
            ctx.fill(Path(CGRect(x: hint.minX, y: 0, width: hint.width, height: 3)), with: .color(.accentColor))
        } else {
            ctx.fill(Path(hint), with: .color(.accentColor.opacity(0.08)))
        }
        if labels {
            ctx.draw(
                Text("likely here").font(.caption2).foregroundStyle(.secondary), at: CGPoint(x: max(hint.minX + 4, 4), y: 4),
                anchor: .topLeading)
        }
        if let press = annotation.pressAt, press >= viewport.start, press <= viewport.end {
            var p = Path()
            p.move(to: CGPoint(x: x(press), y: 0))
            p.addLine(to: CGPoint(x: x(press), y: h))
            let c: Color = onDark ? .white.opacity(0.6) : .secondary.opacity(0.6)
            ctx.stroke(p, with: .color(c), style: StrokeStyle(lineWidth: 1, dash: [2, 3]))
        }
        guard !onDark else { return }
        // Regions under the waveform so the waveform stays readable.
        for r in annotation.regions where r.end >= viewport.start && r.start <= viewport.end {
            let rect = span(r.start, r.end)
            let isSelected = r.id == selected
            ctx.fill(Path(rect), with: .color(r.cls.color.opacity(isSelected ? 0.35 : 0.22)))
            regionEdges(&ctx, rect, r, isSelected)
            if labels, rect.width > 40 {
                ctx.draw(
                    Text(r.cls.title).font(.caption2).foregroundStyle(r.cls.color), at: CGPoint(x: rect.minX + 3, y: h - 3),
                    anchor: .bottomLeading)
            }
        }
    }

    func over(_ ctx: inout GraphicsContext) {
        if onDark {
            // Over the spectrogram, full-height edges would hide the very click they mark: a
            // light tint plus class-coloured bars along the top and bottom edges instead
            // (the selected region also gets a thin outline).
            for r in annotation.regions where r.end >= viewport.start && r.start <= viewport.end {
                let rect = span(r.start, r.end)
                let isSelected = r.id == selected
                let c = r.cls.color
                ctx.fill(Path(rect), with: .color(c.opacity(isSelected ? 0.18 : 0.1)))
                ctx.fill(Path(CGRect(x: rect.minX, y: 0, width: rect.width, height: 4)), with: .color(c))
                ctx.fill(Path(CGRect(x: rect.minX, y: h - 4, width: rect.width, height: 4)), with: .color(c))
                // A light hairline beside each bar keeps dark class colours (plosive, filled
                // pause) visible on the dark image.
                ctx.fill(Path(CGRect(x: rect.minX, y: 4, width: rect.width, height: 1)), with: .color(.white.opacity(0.6)))
                ctx.fill(Path(CGRect(x: rect.minX, y: h - 5, width: rect.width, height: 1)), with: .color(.white.opacity(0.6)))
                if isSelected {
                    ctx.stroke(Path(rect.insetBy(dx: 0.5, dy: 0.5)), with: .color(c.opacity(0.8)), lineWidth: 1)
                }
            }
            // Proposals: dashed brackets along the top and bottom edges.
            for p in annotation.proposals where p.end >= viewport.start && p.start <= viewport.end {
                let decision = annotation.decision(for: p)
                let accepted = decision == .accepted
                let rect = span(p.start, p.end, inset: 6)
                if decision == .rejected {
                    rejectedMark(&ctx, rect)
                    continue
                }
                var path = Path()
                for y in [rect.minY, rect.maxY] {
                    let dy = y == rect.minY ? 5.0 : -5.0
                    path.move(to: CGPoint(x: rect.minX, y: y + dy))
                    path.addLine(to: CGPoint(x: rect.minX, y: y))
                    path.addLine(to: CGPoint(x: rect.maxX, y: y))
                    path.addLine(to: CGPoint(x: rect.maxX, y: y + dy))
                }
                ctx.stroke(
                    path, with: .color(p.cls.color.opacity(accepted ? 0.5 : 1)), style: StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
            }
            return
        }
        // Proposals: dashed outlines.
        for p in annotation.proposals where p.end >= viewport.start && p.start <= viewport.end {
            let decision = annotation.decision(for: p)
            let rect = span(p.start, p.end, inset: 2)
            if decision == .rejected {
                rejectedMark(&ctx, rect)
                continue
            }
            ctx.stroke(
                Path(rect), with: .color(p.cls.color.opacity(decision == .accepted ? 0.4 : 0.9)),
                style: StrokeStyle(lineWidth: 1.2, dash: [4, 3]))
        }
    }

    /// A false positive: a grey dashed box, crossed out.
    private func rejectedMark(_ ctx: inout GraphicsContext, _ rect: CGRect) {
        var cross = Path()
        cross.move(to: CGPoint(x: rect.minX, y: rect.minY))
        cross.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        cross.move(to: CGPoint(x: rect.minX, y: rect.maxY))
        cross.addLine(to: CGPoint(x: rect.maxX, y: rect.minY))
        let grey = GraphicsContext.Shading.color(.gray.opacity(0.8))
        ctx.stroke(Path(rect), with: grey, style: StrokeStyle(lineWidth: 1.2, dash: [4, 3]))
        ctx.stroke(cross, with: grey, lineWidth: 1)
    }

    private func regionEdges(_ ctx: inout GraphicsContext, _ rect: CGRect, _ r: Region, _ isSelected: Bool) {
        if isSelected {
            ctx.stroke(Path(rect.insetBy(dx: 0.5, dy: 0.5)), with: .color(r.cls.color), lineWidth: 1.5)
        }
        for ex in [rect.minX, rect.maxX] {
            ctx.fill(Path(CGRect(x: ex - 0.75, y: 0, width: 1.5, height: h)), with: .color(r.cls.color.opacity(0.9)))
        }
    }
}

// MARK: - Lanes

/// Min/max per pixel column of the mono mix, normalised to the clip peak (gain capped at 20×).
struct WaveformLane: View {
    let model: ReviewModel
    let audio: AudioClip
    let annotation: Annotation
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // The inspector's render of this clip (nil when bypassed: the original has no bands).
        let overlay = model.inspector?.overlay
        Canvas { ctx, size in
            let marks = TimelineMarks(viewport: model.viewport, annotation: annotation, selected: model.selectedRegion, size: size)
            marks.under(&ctx, labels: true)
            drawWave(&ctx, size: size, overlay: overlay)
            if let overlay { drawEvents(&ctx, size: size, events: overlay.events) }
            marks.over(&ctx)
        }
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 6))
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .accessibilityElement()
        .accessibilityLabel("Clip waveform")
        .accessibilityValue(
            Self.summary(
                regions: annotation.regions.count, proposals: annotation.proposals.count, overlay: overlay,
                names: Dictionary(model.inspector?.catalog.map { ($0.id, $0.name) } ?? [], uniquingKeysWith: { a, _ in a })))
    }

    /// "2 marked regions, 1 proposals; removed: De-smack 2, De-ess 5".
    static func summary(regions: Int, proposals: Int, overlay: RenderResult?, names: [String: String]) -> String {
        var parts = ["\(regions) marked regions, \(proposals) proposals"]
        if let overlay {
            let counts = Dictionary(grouping: overlay.events, by: \.module)
            let removed = cleanupModuleKeys.compactMap { key in
                counts[key].map { "\(names[key] ?? key) \($0.count)" }
            }
            parts.append(removed.isEmpty ? "nothing removed" : "removed: " + removed.joined(separator: ", "))
        }
        return parts.joined(separator: "; ")
    }

    /// Min/max per pixel column; with a render, each column is coloured outside-in by what each
    /// module turned down there (`ReductionBands`), the core being what remains.
    private func drawWave(_ ctx: inout GraphicsContext, size: CGSize, overlay: RenderResult?) {
        let w = Double(size.width)
        let mid = Double(size.height) / 2
        let vp = model.viewport
        let cols = max(1, Int(w.rounded(.up)))
        let peaks = audio.peaks(from: vp.start, to: vp.end, columns: cols)
        let peakAbs = audio.peakAbs
        let gain = Double(peakAbs > 0 ? min(20, 0.95 / peakAbs) : 1)
        var wave = Path()
        var paint = BandPainter()
        let colW = w / Double(cols)
        let secondsPerColumn = vp.length / Double(cols)
        for (i, c) in peaks.enumerated() {
            let top = mid - Double(c.max) * gain * mid
            let bottom = mid - Double(c.min) * gain * mid
            let rect = CGRect(x: Double(i) * colW, y: top, width: max(colW, 1), height: max(1, bottom - top))
            if let overlay, c.max >= 0, c.min <= 0 {
                let t0 = vp.start + Double(i) * secondsPerColumn
                let bands = overlay.bands(from: t0, to: t0 + secondsPerColumn)
                if !bands.bands.isEmpty {
                    paint.add(x: rect.minX, width: rect.width, mid: mid, up: mid - top, down: bottom - mid, bands: bands)
                    continue
                }
            }
            wave.addRect(rect)
        }
        let core = Color.primary.opacity(0.7)
        ctx.fill(wave, with: .color(core))
        paint.fill(&ctx, scheme: scheme, core: core)
    }

    /// What the render removed, as short ticks along the top edge in the module's colour.
    private func drawEvents(_ ctx: inout GraphicsContext, size: CGSize, events: [RenderEvent]) {
        let vp = model.viewport
        let w = Double(size.width)
        for e in events where e.start + e.duration >= vp.start && e.start <= vp.end && e.module != "limiter" {
            let x0 = vp.x(at: e.start, width: w)
            let x1 = vp.x(at: e.start + e.duration, width: w)
            let rect = CGRect(x: x0, y: 0, width: max(2, x1 - x0), height: 3)
            ctx.fill(Path(roundedRect: rect, cornerRadius: 1), with: .color(ModuleStyle.color(e.module, scheme).opacity(0.9)))
        }
    }
}

/// Log-frequency spectrogram (30 Hz … 20 kHz, magma, 80 dB below the clip's loudest bin).
/// Computed off the main thread per clip and resolution (`SpectrogramCache`); at deep zoom a
/// sharper render of the visible stretch replaces the overview once ready.
struct SpectrogramLane: View {
    let model: ReviewModel
    let audio: AudioClip
    let annotation: Annotation
    let resolution: SpectrogramResolution
    let width: Double
    @Environment(\.displayScale) private var displayScale

    private var cache: SpectrogramCache { model.spectrograms }
    private var clipID: String { model.current?.stem ?? "" }
    private var overviewKey: SpectrogramCache.Key { .init(clip: clipID, resolution: resolution) }
    private var detailKey: SpectrogramCache.Key? {
        SpectrogramDetail.plan(
            viewport: model.viewport, pixels: width * displayScale, sampleRate: audio.sampleRate,
            overviewHop: resolution.hop(sampleRate: audio.sampleRate),
            fftSize: resolution.fftSize(sampleRate: audio.sampleRate), clipSamples: audio.mono.count
        ).map { .init(clip: clipID, resolution: resolution, detail: $0) }
    }

    var body: some View {
        let overview = cache.image(overviewKey)
        let detail = detailKey.flatMap { key in
            cache.image(key)
                ?? cache.latestDetail(clip: clipID, resolution: resolution, covering: model.viewport.start...model.viewport.end)
        }
        Canvas { ctx, size in
            let vp = model.viewport
            let rect = CGRect(origin: .zero, size: size)
            ctx.fill(Path(rect), with: .color(Self.floorColor))
            let marks = TimelineMarks(
                viewport: vp, annotation: annotation, selected: model.selectedRegion, size: size, onDark: true)
            if let img = detail ?? overview {
                draw(img, in: &ctx, size: size, viewport: vp)
            } else {
                ctx.draw(
                    Text("Computing spectrogram…").font(.caption).foregroundStyle(.white.opacity(0.7)),
                    at: CGPoint(x: size.width / 2, y: size.height / 2))
            }
            marks.under(&ctx, labels: false)
            marks.over(&ctx)
            drawFrequencyTicks(&ctx, size: size, scale: (detail ?? overview)?.scale ?? .speech(sampleRate: audio.sampleRate))
        }
        .clipShape(RoundedRectangle(cornerRadius: 6))
        // `ready` in the id: if the overview were ever evicted, it is computed again.
        .task(id: TaskID(key: overviewKey, ready: overview != nil)) {
            guard overview == nil else { return }
            await cache.load(overviewKey, audio: audio)
        }
        .task(id: TaskID(key: detailKey, ready: overview != nil)) {
            guard let key = detailKey, let overview else { return }
            // Let a zoom/pan gesture settle before computing.
            try? await Task.sleep(for: .milliseconds(80))
            guard !Task.isCancelled else { return }
            await cache.load(key, audio: audio, top: overview.topDb)
        }
        .accessibilityElement()
        .accessibilityLabel("Spectrogram")
        .accessibilityValue(overview == nil ? "Computing" : accessibilityDescription)
    }

    private var accessibilityDescription: String {
        (
            "Log frequency from 30 hertz to \(Int(FrequencyScale.speech(sampleRate: audio.sampleRate).maxHz / 1000)) kilohertz, "
                + "\(resolution.title.lowercased()). Clicks and smacks show as bright vertical lines, hum as a low horizontal line, "
                + "sibilance as energy above 5 kilohertz.")
    }

    private struct TaskID: Hashable {
        var key: SpectrogramCache.Key?
        var ready: Bool
    }

    /// Magma's darkest colour, behind the image (and while computing).
    static let floorColor = Color(
        .sRGB, red: Double(Colormap.magma[0]) / 255, green: Double(Colormap.magma[1]) / 255,
        blue: Double(Colormap.magma[2]) / 255)

    private func draw(_ img: SpectrogramImage, in ctx: inout GraphicsContext, size: CGSize, viewport vp: Viewport) {
        guard img.slice(from: vp.start, to: vp.end) != nil else { return }
        let w = Double(size.width)
        let x0 = vp.x(at: img.extent.lowerBound, width: w)
        let x1 = vp.x(at: img.extent.upperBound, width: w)
        // Columns wider than 2 device pixels: show them as blocks (honest about time
        // resolution) until the detail render arrives; otherwise smooth.
        let columnPixels = (x1 - x0) / Double(img.frames) * displayScale
        let image = Image(decorative: img.image, scale: 1).interpolation(columnPixels > 2 ? .none : .medium)
        ctx.draw(image, in: CGRect(x: x0, y: 0, width: x1 - x0, height: Double(size.height)))
    }

    private func drawFrequencyTicks(_ ctx: inout GraphicsContext, size: CGSize, scale: FrequencyScale) {
        let h = Double(size.height)
        var lastY = -Double.infinity
        // Labelled ticks, top down; skip any that would crowd the previous label.
        for hz in scale.ticks.reversed() {
            let y = scale.row(ofHz: hz, rows: 1) * h
            guard y - lastY >= 14, y > 6, y < h - 4 else { continue }
            lastY = y
            ctx.fill(Path(CGRect(x: 0, y: y - 0.25, width: 5, height: 0.5)), with: .color(.white.opacity(0.8)))
            let label = ctx.resolve(Text(FrequencyScale.label(hz)).font(.caption2.monospacedDigit()).foregroundStyle(.white))
            let ls = label.measure(in: size)
            let box = CGRect(x: 6, y: y - ls.height / 2 - 1, width: ls.width + 6, height: ls.height + 2)
            ctx.fill(Path(roundedRect: box, cornerRadius: 3), with: .color(.black.opacity(0.65)))
            ctx.draw(label, at: CGPoint(x: box.midX, y: y))
        }
    }
}

/// The playhead across all lanes; the only view redrawn at display rate while playing. While
/// playing it shows where playback is and, when zoomed in, pages the view along with it.
struct PlayheadOverlay: View {
    let model: ReviewModel

    var body: some View {
        let playing = model.player?.isPlaying == true
        TimelineView(.animation(paused: !playing)) { _ in
            // Read the time here, outside the Canvas, and hand it in as an input: a Canvas whose
            // inputs don't change isn't redrawn on each timeline tick, so the line stood still.
            let t = (playing ? model.player?.currentTime : nil) ?? model.playhead
            PlayheadLine(time: t, viewport: model.viewport)
                .onChange(of: t) { _, now in
                    guard model.player?.isPlaying == true, let next = model.viewport.following(now) else { return }
                    model.viewport = next
                }
        }
    }
}

/// One vertical accent line at `time`.
struct PlayheadLine: View {
    let time: Double
    let viewport: Viewport

    var body: some View {
        Canvas { ctx, size in
            guard time >= viewport.start, time <= viewport.end else { return }
            let x = viewport.x(at: time, width: Double(size.width))
            ctx.fill(Path(CGRect(x: x - 0.75, y: 0, width: 1.5, height: size.height)), with: .color(.accentColor))
        }
        .accessibilityHidden(true)
    }
}

/// Seconds (or ms when zoomed in) under the lanes.
struct TimeAxis: View {
    let viewport: Viewport

    var body: some View {
        Canvas { ctx, size in
            let vp = viewport
            let w = Double(size.width)
            ctx.fill(Path(CGRect(x: 0, y: 0, width: w, height: 0.5)), with: .color(.secondary.opacity(0.5)))
            let step = TimelineEditor.niceStep(vp.length / max(w, 1) * 90)
            let useMs = step < 0.1
            var t = (vp.start / step).rounded(.up) * step
            while t <= vp.end {
                let px = vp.x(at: t, width: w)
                ctx.fill(Path(CGRect(x: px, y: 0, width: 0.5, height: 4)), with: .color(.secondary))
                let label = useMs ? String(format: "%.0f ms", t * 1000) : String(format: "%.1f s", t)
                ctx.draw(
                    Text(label).font(.caption2.monospacedDigit()).foregroundStyle(.secondary), at: CGPoint(x: px + 2, y: 3),
                    anchor: .topLeading)
                t += step
            }
        }
    }
}

/// Catches scroll-wheel and pinch events over its frame (SwiftUI has no scroll-wheel gesture on
/// macOS) without taking mouse clicks: it is a background view with a local event monitor.
struct ScrollZoomCatcher: NSViewRepresentable {
    /// (dx, dy, ⌘ held, precise trackpad deltas, x as a fraction of the width)
    var onScroll: @MainActor (Double, Double, Bool, Bool, Double) -> Void
    /// (magnification delta, x fraction)
    var onMagnify: @MainActor (Double, Double) -> Void

    func makeNSView(context: Context) -> CatcherView {
        let v = CatcherView()
        v.onScroll = onScroll
        v.onMagnify = onMagnify
        return v
    }

    func updateNSView(_ v: CatcherView, context: Context) {
        v.onScroll = onScroll
        v.onMagnify = onMagnify
    }

    static func dismantleNSView(_ v: CatcherView, coordinator: ()) { v.removeMonitor() }

    final class CatcherView: NSView {
        var onScroll: (@MainActor (Double, Double, Bool, Bool, Double) -> Void)?
        var onMagnify: (@MainActor (Double, Double) -> Void)?
        private var monitor: Any?

        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            removeMonitor()
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: [.scrollWheel, .magnify]) { [weak self] event in
                let isScroll = event.type == .scrollWheel
                let dx = Double(isScroll ? event.scrollingDeltaX : 0)
                let dy = Double(isScroll ? event.scrollingDeltaY : 0)
                let precise = isScroll && event.hasPreciseScrollingDeltas
                let cmd = event.modifierFlags.contains(.command)
                let mag = Double(isScroll ? 0 : event.magnification)
                let loc = event.locationInWindow
                let windowNumber = event.windowNumber
                let handled = MainActor.assumeIsolated { () -> Bool in
                    guard let self, let window = self.window, window.windowNumber == windowNumber else { return false }
                    let p = self.convert(loc, from: nil)
                    guard self.bounds.contains(p), self.bounds.width > 0 else { return false }
                    let frac = Double(p.x / self.bounds.width)
                    if isScroll { self.onScroll?(dx, dy, cmd, precise, frac) } else { self.onMagnify?(mag, frac) }
                    return true
                }
                return handled ? nil : event
            }
        }

        func removeMonitor() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }
    }
}

// PreviewProvider rather than #Preview (no macro plugin with Command Line Tools).
struct TimelineEditor_Previews: PreviewProvider {
    static var previews: some View {
        let m = ReviewModel.preview()
        return ForEach([true, false], id: \.self) { spectrogram in
            if let audio = m.audio, let a = m.annotation {
                TimelineEditor(model: m, audio: audio, annotation: a, showSpectrogram: spectrogram, resolution: .balanced, refocus: {})
                    .frame(width: 700, height: spectrogram ? 340 : 200)
                    .padding()
            }
        }
    }
}
