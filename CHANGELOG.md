# Changelog

All notable changes to hEARo are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) from 1.0 on. The workspace version
is 0.1.0.

## Unreleased

### Added

- `hearo dataset eval`: score a detector against your reviewed snapshots (regions found,
  false positives, catches you rejected in review).

### Changed

- De-smack leaves quiet and dull bursts alone: a smack must peak within *Below speech* (new
  setting, 18 dB) of the speech level and have ≥ 16 % of its energy above 1.5 kHz. On 106 reviewed clips this
  halves the false positives (126 → 60) for 3 of 37 smacks.

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
