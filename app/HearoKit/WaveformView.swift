import SwiftUI

/// Live waveform: the sound scrolls in from the right. Where a cleanup module turned something
/// down, the outer part of the stroke is painted in the module's colour, as deep as the cut
/// (`ReductionBands`); the grey core is what is playing.
public struct WaveformView: View {
    let buffer: WaveformBuffer
    /// Module display names by key (VoiceOver summary); falls back to the key.
    let names: [String: String]
    @Environment(\.colorScheme) private var scheme

    public init(buffer: WaveformBuffer, names: [String: String] = [:]) {
        self.buffer = buffer
        self.names = names
    }

    public var body: some View {
        Canvas { ctx, size in
            let cols = buffer.columns
            let w = size.width / CGFloat(max(buffer.capacity, 1))
            let mid = size.height / 2
            // Right-aligned: the newest column touches the right edge.
            let x0 = size.width - CGFloat(cols.count) * w
            var paint = BandPainter()
            for (i, c) in cols.enumerated() {
                let h = max(0.5, CGFloat(WaveformBuffer.height(c.peak)) * mid)
                let bands = ReductionBands.compute(
                    peak: c.peak, outPeak: c.reductionDb.isEmpty ? nil : c.outPeak, reductionsDb: c.reductionDb)
                paint.add(x: x0 + CGFloat(i) * w, width: max(w * 0.8, 0.8), mid: mid, up: h, down: h, bands: bands)
            }
            paint.fill(&ctx, scheme: scheme, core: .secondary.opacity(0.75))
            // Centre line so silence still reads as "live".
            ctx.fill(Path(CGRect(x: 0, y: mid - 0.25, width: size.width, height: 0.5)), with: .color(.secondary.opacity(0.3)))
        }
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 6))
        .accessibilityElement()
        .accessibilityLabel("Live waveform")
        .accessibilityValue(Self.summary(buffer, names: names))
    }

    /// "Recently turned down: De-smack up to 18 dB, De-ess up to 8 dB" (deepest cut per module).
    static func summary(_ buffer: WaveformBuffer, names: [String: String]) -> String {
        var deepest = [Float](repeating: 0, count: cleanupModuleKeys.count)
        for c in buffer.columns {
            for (k, db) in c.reductionDb.enumerated() where k < deepest.count && db >= 1 { deepest[k] = max(deepest[k], db) }
        }
        let parts = zip(cleanupModuleKeys, deepest).filter { $0.1 > 0 }.map { key, db in
            "\(names[key] ?? key) up to \(Int(db.rounded())) dB"
        }
        if parts.isEmpty {
            return buffer.columns.contains(where: \.removed) ? "Recently removed sounds" : "Nothing removed recently"
        }
        return "Recently turned down: " + parts.joined(separator: ", ")
    }
}

/// Collects column rectangles into one path per colour and fills them in a few calls (cheap at
/// 60 columns/s). Shared by the live waveform and the review window's waveform lane.
struct BandPainter {
    private var core = Path()
    private var modules = [Path](repeating: Path(), count: cleanupModuleKeys.count)

    /// One column at `x`: `up`/`down` = the original stroke's extent above/below `mid`.
    /// A band of a module that cut ≥ 1 dB is at least 1 pt thick so it never vanishes.
    mutating func add(x: CGFloat, width: CGFloat, mid: CGFloat, up: CGFloat, down: CGFloat, bands: ReductionBands) {
        var upEdge = up  // distance from mid of the current outer edge
        var downEdge = down
        for b in bands.bands where b.module < modules.count {
            let tUp = max(up * CGFloat(b.thickness), b.db >= 1 ? 1 : 0)
            let tDown = max(down * CGFloat(b.thickness), b.db >= 1 ? 1 : 0)
            let upInner = max(0, upEdge - tUp)
            let downInner = max(0, downEdge - tDown)
            if upEdge > upInner { modules[b.module].addRect(CGRect(x: x, y: mid - upEdge, width: width, height: upEdge - upInner)) }
            if downEdge > downInner {
                modules[b.module].addRect(CGRect(x: x, y: mid + downInner, width: width, height: downEdge - downInner))
            }
            upEdge = upInner
            downEdge = downInner
        }
        let coreUp = min(upEdge, up * CGFloat(bands.core))
        let coreDown = min(downEdge, down * CGFloat(bands.core))
        if coreUp + coreDown > 0 {
            core.addRect(CGRect(x: x, y: mid - coreUp, width: width, height: max(0.5, coreUp + coreDown)))
        }
    }

    func fill(_ ctx: inout GraphicsContext, scheme: ColorScheme, core coreColor: Color, opacity: Double = 1) {
        ctx.fill(core, with: .color(coreColor))
        for (k, p) in modules.enumerated() where !p.isEmpty {
            ctx.fill(p, with: .color(ModuleStyle.color(cleanupModuleKeys[k], scheme).opacity(opacity)))
        }
    }
}

struct WaveformView_Previews: PreviewProvider {
    static var previews: some View {
        var b = WaveformBuffer(capacity: 180)
        b.append(FakeEngine.syntheticWaveform(columns: 180, start: 0))
        return WaveformView(buffer: b).frame(width: 332, height: 56).padding()
    }
}
