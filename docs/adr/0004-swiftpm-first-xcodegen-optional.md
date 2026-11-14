# 0004. SwiftPM-first app build; XcodeGen is the optional IDE path

Date: 2026-11-13 · Status: accepted

## Context

The brief assumed XcodeGen + `xcodebuild`. The dev machine initially had only Command Line
Tools, which cannot run `xcodebuild` builds, Swift Testing, `#Preview` macros or Instruments,
but can build SwiftUI apps with SwiftPM and sign them with `codesign`.

## Decision

All UI code lives in the SwiftPM package `HearoKit` (`app/Package.swift`); a thin `@main`
target wraps it. `just app` builds with SwiftPM, assembles the `.app` bundle itself
(`scripts/build-app.sh`), signs it (`scripts/sign.sh`) and installs it. `app/project.yml`
(XcodeGen) remains for working in Xcode (`just xcode`) and is validated by `just ci`. Previews
use `PreviewProvider` (no macro). Swift tests run whenever Xcode is installed.

## Alternatives considered

- **Xcode-only** — blocks all app work until Xcode is installed; `.pbxproj` churn.
- **Tuist** — another tool to pin; XcodeGen is enough for one target.

## Consequences (incl. how we'd know it was wrong)

- Two build descriptions (Package.swift, project.yml) must agree; `just ci` builds one and
  generates the other.
- Wrong if: resources/asset catalogs or the XCFramework (M3) can't be handled by the SwiftPM
  path without large custom scripting — then switch `just app` to `xcodebuild`.
