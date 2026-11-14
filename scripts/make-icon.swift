// Render the hEARo app icon from its SVG sources into an .iconset and an .icns.
// Usage: swift scripts/make-icon.swift <master.svg> <small.svg> <out.icns>
// Pixel sizes <= 32 use the small-size variant; everything else uses the master. The .iconset
// is kept next to the .icns so the PNGs can be inspected. Needs only Command Line Tools:
// AppKit renders SVG natively (CoreSVG), iconutil packs the .icns. Run via `just icon`.
import AppKit

let args = CommandLine.arguments
guard args.count == 4 else {
    FileHandle.standardError.write(Data("usage: make-icon.swift <master.svg> <small.svg> <out.icns>\n".utf8))
    exit(2)
}

func load(_ path: String) -> NSImage {
    guard let image = NSImage(contentsOf: URL(fileURLWithPath: path)), image.size.width > 0 else {
        FileHandle.standardError.write(Data("✘ cannot load \(path)\n".utf8))
        exit(1)
    }
    return image
}

let master = load(args[1])
let small = load(args[2])
let icns = URL(fileURLWithPath: args[3])
let iconset = icns.deletingPathExtension().appendingPathExtension("iconset")

func png(_ image: NSImage, pixels n: Int) -> Data {
    let space = CGColorSpace(name: CGColorSpace.sRGB)!
    let ctx = CGContext(
        data: nil, width: n, height: n, bitsPerComponent: 8, bytesPerRow: 0, space: space,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .high
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)
    image.draw(in: NSRect(x: 0, y: 0, width: n, height: n))
    NSGraphicsContext.restoreGraphicsState()
    let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
    return rep.representation(using: .png, properties: [:])!
}

let fm = FileManager.default
try? fm.removeItem(at: iconset)
try fm.createDirectory(at: iconset, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = points * scale
        let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
        try png(pixels <= 32 ? small : master, pixels: pixels).write(to: iconset.appendingPathComponent(name))
    }
}

let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", icns.path]
try iconutil.run()
iconutil.waitUntilExit()
guard iconutil.terminationStatus == 0 else { exit(iconutil.terminationStatus) }
print("✔ \(icns.path)")
