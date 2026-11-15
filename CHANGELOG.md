# Changelog

All notable changes to hEARo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) from 1.0 on. Every user-visible
change adds a line under *Unreleased* in the same pull request; a release renames that
section ([docs/dev/RELEASING.md](docs/dev/RELEASING.md)).

## Unreleased

## [0.2.0] - 2026-11-15

### Added

- `hearo dataset eval`: score a detector against your reviewed snapshots (regions found,
  false positives, catches you rejected in review).
- Review: a caption under the *Reviewed* toggle reminds you to mark every mouth noise first,
  because the unmarked rest of a reviewed clip counts as clean audio for training.

### Changed

- De-smack leaves quiet and dull bursts alone: a smack must peak within *Below speech* (new
  setting, 22 dB) of the speech level and have ≥ 10 % of its energy above 1.5 kHz. On 111
  reviewed clips: false positives 132 → 81 with the same 37 smacks found.
- The settings window is called "Settings…" everywhere (it was "Advanced settings…" and
  "Advanced…" in some menus), and the ⋯ menu shows its ⌘, shortcut.

### Fixed

- With global bypass on, the popover's trigger count and the menu bar's "removing" glow no
  longer move, and modules only report clicks and smacks they actually removed (none while
  fading in or out, none after a fault).
- Review: flipping *Reviewed* no longer shifts the buttons in the header.

## [0.1.0] - 2026-11-15

### Added

- **DSP modules** (`hearo-dsp`), each with golden audio tests:
  de-hum (mains tracking, harmonic gating), de-plosive (dynamic low cut), de-click (LPC
  detection + AR interpolation), de-smack (pause-aware lip-smack removal with 40 ms
  lookahead), de-ess (split-band dynamic EQ) and a true-peak safety limiter.
- **DSP foundation:** planar audio blocks, `Module` trait, a chain with constant latency
  compensation, click-free toggles, bit-exact global bypass and fault isolation (non-finite
  samples, denormals); streaming STFT analysis.
- **Live engine** (`hearo-engine`): Core Audio process tap (global minus exclusions) with a
  private aggregate device and a real-time IOProc; idle detection and permission check;
  follows the system output device (connect/disconnect, sample-rate change); 64-frame IO
  buffer keeps the full chain under the 45 ms latency ceiling.
- **Menu bar app** (SwiftUI) driving the Rust engine through a UniFFI control plane: master
  switch and module toggles (remembered across launches), live waveform with removed sounds
  in red, activity icon, launch at login, right-click app menu.
- **Labeling snapshots:** a global `§` hotkey saves the last seconds of audio locally; a
  review window to play, annotate and delete them; export/import to combine snapshots
  from several Macs.
- **Developer CLI** (`hearo`): test-signal generation, offline processing with reports,
  inspection, live `procs`/`run`, and `dataset propose` for detector-generated weak labels.
- **Test kit** (`hearo-testkit`): seeded test signals, quality metrics, WAV I/O and an
  allocation counter for real-time-safety tests.
- **Tooling:** pinned toolchains (mise), local CI via `just check`/`just ci` and lefthook
  hooks (no hosted CI, ADR 0009), signed DMG build (`just release`, unnotarized), an `ml/`
  Python project skeleton, architecture decision records 0001–0013 and deep dives.
- Open-source readiness: MIT OR Apache-2.0 licence texts, contributing guide, code of
  conduct, security policy, issue and PR templates, documentation index.
- **GitHub Actions** (ADR 0016, supersedes 0009): `just ci` and Rust coverage (`just
  coverage`, Codecov) on every pull request; tagged releases build an ad-hoc signed DMG with a
  checksum and build-provenance attestation and publish it as a GitHub Release. Actions are
  pinned by commit SHA; Dependabot keeps actions and crates current. README with icon, badges
  and screenshots.
