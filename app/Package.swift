// swift-tools-version: 6.2
// HearoKit: all UI (views, view models, Engine protocol, FakeEngine) — builds and tests without
// Rust. HearoLive: RustEngine (UniFFI bindings to the Rust engine) + global hotkey. The Rust side
// is built by `just ffi` into RustBridge/ (generated, git-ignored). See ADR 0002 / ADR 0004.
import PackageDescription

let package = Package(
    name: "HearoKit",
    platforms: [.macOS("26.0")],
    products: [
        .library(name: "HearoKit", targets: ["HearoKit"]),
        .library(name: "HearoLive", targets: ["HearoLive"]),
    ],
    targets: [
        .target(name: "HearoKit", path: "HearoKit"),
        .binaryTarget(name: "hearo_ffiFFI", path: "RustBridge/HearoFFI.xcframework"),
        .target(
            name: "HearoFFI",
            dependencies: ["hearo_ffiFFI"],
            path: "RustBridge/generated",
            sources: ["hearo_ffi.swift"],
            swiftSettings: [.swiftLanguageMode(.v5)],  // generated code
            linkerSettings: [
                .linkedFramework("CoreAudio"),
                .linkedFramework("AudioToolbox"),
                .linkedFramework("CoreFoundation"),
                .linkedFramework("Foundation"),
            ]
        ),
        .target(name: "HearoLive", dependencies: ["HearoKit", "HearoFFI"], path: "HearoLive"),
        .executableTarget(name: "hEARoApp", dependencies: ["HearoKit", "HearoLive"], path: "hEARo"),
        .testTarget(name: "HearoKitTests", dependencies: ["HearoKit"], path: "hEARoTests"),
    ]
)
