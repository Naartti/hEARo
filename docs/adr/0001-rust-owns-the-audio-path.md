# 0001. Rust owns the entire audio path

Date: 2026-11-13 · Status: accepted

## Context

hEARo captures system audio with Core Audio process taps, processes it under hard real-time
constraints, and plays it back. The UI is a SwiftUI menu bar app. Agents must be able to build,
test and benchmark the audio path headlessly.

## Decision

Rust owns everything on the audio path: tap and aggregate-device setup, the IOProc, DSP, ML
inference, telemetry production. Swift owns only the UI and talks to Rust through a control
plane (ADR 0002).

## Alternatives considered

- **Swift/AVAudioEngine for capture, Rust for DSP** — two languages on the RT thread, FFI calls
  per block, harder to test without a GUI.
- **All Swift** — no `realfft`/`tract`, weaker allocation discipline tooling, no headless CLI.

## Consequences (incl. how we'd know it was wrong)

- The `hearo` CLI exercises the real engine without the app; tests run with `cargo nextest`.
- We depend on `objc2-core-audio`/`coreaudio` bindings for newer APIs (process taps).
- Wrong if: tap APIs prove unusable from Rust (missing bindings we can't write) or the FFI
  boundary forces per-block Swift↔Rust calls.
