import Foundation
import Testing

@testable import HearoKit

/// Module colours, slider formatting, outside-in bands, the settings store and the review
/// inspector (model level; views are checked by rendering them, see docs/DESIGN.md).
@MainActor
struct ModuleStyleTests {
    @Test func everyCleanupModuleHasItsOwnColourInBothAppearances() {
        let styles = cleanupModuleKeys.map(ModuleStyle.style)
        #expect(styles.map(\.key) == cleanupModuleKeys)
        #expect(Set(styles.map(\.light)).count == cleanupModuleKeys.count)
        #expect(Set(styles.map(\.dark)).count == cleanupModuleKeys.count)
        #expect(Set(styles.map(\.colorName)).count == cleanupModuleKeys.count, "names tell colours apart in tooltips")
        #expect(styles.allSatisfy { $0.isCleanup })
        #expect(!ModuleStyle.style("limiter").isCleanup)
        #expect(ModuleStyle.style("future-module").colorName == "grey")
    }

    /// Dark variants are lighter (they sit on dark backgrounds and the magma spectrogram).
    @Test func darkVariantsAreLighter() {
        func luma(_ hex: UInt32) -> Double {
            0.2126 * Double((hex >> 16) & 0xFF) + 0.7152 * Double((hex >> 8) & 0xFF) + 0.0722 * Double(hex & 0xFF)
        }
        for s in ModuleStyle.all where s.isCleanup { #expect(luma(s.dark) > luma(s.light), "\(s.key)") }
    }

    /// WCAG 1.4.11: band colours are ≥ 3:1 against the window background in both appearances.
    @Test func bandColoursMeetNonTextContrast() {
        func lum(_ hex: UInt32) -> Double {
            func lin(_ c: UInt32) -> Double {
                let v = Double(c & 0xFF) / 255
                return v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
            }
            return 0.2126 * lin(hex >> 16) + 0.7152 * lin(hex >> 8) + 0.0722 * lin(hex)
        }
        func ratio(_ a: UInt32, _ b: UInt32) -> Double {
            let (x, y) = (lum(a), lum(b))
            return (max(x, y) + 0.05) / (min(x, y) + 0.05)
        }
        for s in ModuleStyle.all where s.isCleanup {
            #expect(ratio(s.light, 0xECECEC) >= 3, "\(s.key) light")
            #expect(ratio(s.dark, 0x2A2A2A) >= 3, "\(s.key) dark")
        }
    }
}

/// Outside-in bands: the geometry behind the live and review waveform colours.
struct ReductionBandsTests {
    /// Index of each module in `cleanupModuleKeys`.
    let dehum = 0, deplosive = 1, declick = 2, desmack = 3, deess = 4

    private func row(_ pairs: [Int: Float]) -> [Float] {
        (0..<cleanupModuleKeys.count).map { pairs[$0] ?? 0 }
    }

    private func close(_ a: Float, _ b: Float, _ tol: Float = 1e-5) -> Bool { abs(a - b) <= tol }

    @Test func noReductionIsAPlainWaveform() {
        #expect(ReductionBands.compute(peak: 0.5, outPeak: 0.5, reductionsDb: row([:])) == .none)
        #expect(ReductionBands.compute(peak: 0.5, outPeak: nil, reductionsDb: []) == .none)
        #expect(ReductionBands.compute(peak: 0, outPeak: 0, reductionsDb: row([desmack: 12])) == .none, "silence")
        #expect(
            ReductionBands.compute(peak: 0.5, outPeak: 0.5, reductionsDb: row([dehum: 0.25])) == .none,
            "below the noise floor")
    }

    /// The owner's example: −12 dB de-smack colours most of the stroke, −2 dB de-ess a thin rim.
    @Test func depthFollowsTheAmplitudeRemoved() {
        let smack = ReductionBands.compute(peak: 0.5, outPeak: nil, reductionsDb: row([desmack: 12]))
        #expect(close(smack.core, pow(10, -12 / 20)))
        #expect(smack.bands.count == 1 && smack.bands[0].module == desmack && smack.bands[0].thickness > 0.7)
        let ess = ReductionBands.compute(peak: 0.5, outPeak: nil, reductionsDb: row([deess: 2]))
        #expect(ess.bands.count == 1 && ess.bands[0].thickness < 0.25)
        #expect(ess.bands[0].thickness < smack.bands[0].thickness / 3)
    }

    @Test func bandsTileTheRimOutsideInInChainOrder() {
        let b = ReductionBands.compute(peak: 0.8, outPeak: nil, reductionsDb: row([dehum: 3, desmack: 9, deess: 6]))
        #expect(b.bands.map(\.module) == [dehum, desmack, deess])
        #expect(b.bands.first?.outer == 1)
        #expect(b.bands.last?.inner == b.core)
        for (a, c) in zip(b.bands, b.bands.dropFirst()) { #expect(a.inner == c.outer, "no gaps or overlaps") }
        let sum = b.bands.reduce(Float(0)) { $0 + $1.thickness }
        #expect(close(sum, 1 - b.core), "bands sum to envelope − core")
        // Split in proportion to dB: de-smack's 9 dB is 3× de-hum's 3 dB.
        #expect(close(b.bands[1].thickness, 3 * b.bands[0].thickness))
    }

    /// Property-like sweep: more dB on one module never shrinks its band or grows the core.
    @Test func monotoneInDecibels() {
        for other: Float in [0, 2, 6] {
            var lastCore: Float = 1
            var lastBand: Float = 0
            for db in stride(from: Float(0), through: 40, by: 0.5) {
                let b = ReductionBands.compute(peak: 0.6, outPeak: nil, reductionsDb: row([deplosive: db, deess: other]))
                let band = b.bands.first { $0.module == deplosive }?.thickness ?? 0
                #expect(b.core <= lastCore + 1e-6, "core shrinks as \(db) dB grows")
                #expect(band >= lastBand - 1e-6, "band grows with \(db) dB")
                #expect(b.core >= 0 && b.core <= 1)
                lastCore = b.core
                lastBand = band
            }
        }
    }

    /// The measured output peak is preferred, clamped to the input peak, and never paints more
    /// than the modules report (e.g. limiter gain is not a cleanup module's colour).
    @Test func measuredOutputPeakWinsWithinTheReportedReduction() {
        let measured = ReductionBands.compute(peak: 0.5, outPeak: 0.4, reductionsDb: row([desmack: 12]))
        #expect(close(measured.core, 0.8), "the output kept 80 %")
        let limited = ReductionBands.compute(peak: 0.5, outPeak: 0.05, reductionsDb: row([deess: 2]))
        #expect(close(limited.core, pow(10, -2 / 20)), "at most what de-ess reports")
        let louder = ReductionBands.compute(peak: 0.5, outPeak: 0.7, reductionsDb: row([deess: 6]))
        #expect(louder == .none, "output ≥ input: nothing to colour")
        let nan = ReductionBands.compute(peak: 0.5, outPeak: .nan, reductionsDb: row([deess: 6, declick: .infinity]))
        #expect(nan.bands.map(\.module) == [deess], "non-finite values are ignored")
    }

    @Test func tinyPeaksStillGetTheirBands() {
        let b = ReductionBands.compute(peak: 1e-4, outPeak: 2.5e-5, reductionsDb: row([declick: 12]))
        #expect(b.bands.count == 1 && close(b.core, 0.25, 2e-3))
    }
}
