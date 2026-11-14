import SwiftUI

/// One colour per cleanup module, defined once: the popover module list (which doubles as the
/// legend), the live waveform bands, the settings window and the review window all use it.
///
/// Based on the Okabe–Ito colour-blind-safe palette, with lightness spread (L* 33–54 in light
/// mode, 58–82 in dark mode) so neighbours stay apart under protanopia, deuteranopia and
/// tritanopia, not just by hue (minimum simulated ΔE ≈ 13). Light variants are ≥ 3:1 against
/// light window backgrounds (WCAG 1.4.11); dark-mode variants are lighter so they read on dark
/// window backgrounds and on the magma spectrogram. Colour is never the only
/// cue: every swatch sits next to the module's name, and VoiceOver reads names and dB values.
public struct ModuleStyle: Sendable, Equatable {
    public let key: String
    /// sRGB hex for light and dark appearance.
    public let light: UInt32
    public let dark: UInt32
    /// Plain-language colour name (tooltips, VoiceOver hints).
    public let colorName: String

    /// Cleanup modules in chain order (`cleanupModuleKeys`), then the limiter (neutral grey:
    /// it has no reduction row and is never drawn as a band).
    public static let all: [ModuleStyle] = [
        ModuleStyle(key: "dehum", light: 0x0A4F8C, dark: 0x3B8FD9, colorName: "blue"),
        ModuleStyle(key: "deplosive", light: 0xB8509A, dark: 0xE08DBF, colorName: "pink"),
        ModuleStyle(key: "declick", light: 0xA67200, dark: 0xF2B233, colorName: "amber"),
        ModuleStyle(key: "desmack", light: 0x8F2A00, dark: 0xF2662A, colorName: "orange-red"),
        ModuleStyle(key: "deess", light: 0x2B86C5, dark: 0x8FD4F7, colorName: "sky blue"),
        ModuleStyle(key: "limiter", light: 0x8E8E93, dark: 0x98989D, colorName: "grey"),
    ]

    /// Unknown modules (a newer engine) get the neutral grey.
    public static func style(_ key: String) -> ModuleStyle {
        all.first(where: { $0.key == key })
            ?? ModuleStyle(key: key, light: 0x8E8E93, dark: 0x98989D, colorName: "grey")
    }

    public static func color(_ key: String, _ scheme: ColorScheme) -> Color { style(key).color(scheme) }

    public func color(_ scheme: ColorScheme) -> Color {
        let hex = scheme == .dark ? dark : light
        return Color(
            .sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255)
    }

    /// For the menu bar icon: the menu bar's own appearance follows the wallpaper, not the app,
    /// so one mid-luminance variant (the dark-appearance one) that reads on light and dark bars.
    public var menuBarHex: UInt32 { dark }

    public var menuBarColor: Color { color(.dark) }

    /// Whether the module is drawn as a band (has a gain-reduction row).
    public var isCleanup: Bool { cleanupModuleKeys.contains(key) }
}

/// A small rounded colour chip for a module (the legend). Hidden from VoiceOver: the row it sits
/// in carries the module name.
public struct ModuleSwatch: View {
    let key: String
    var size: CGFloat = 10
    @Environment(\.colorScheme) private var scheme

    public init(_ key: String, size: CGFloat = 10) {
        self.key = key
        self.size = size
    }

    public var body: some View {
        let style = ModuleStyle.style(key)
        RoundedRectangle(cornerRadius: size * 0.3)
            .fill(style.color(scheme))
            .overlay(RoundedRectangle(cornerRadius: size * 0.3).strokeBorder(.primary.opacity(0.15), lineWidth: 0.5))
            .frame(width: size, height: size)
            .help(style.isCleanup ? "Shown in \(style.colorName) in the waveform" : "")
            .accessibilityHidden(true)
    }
}
