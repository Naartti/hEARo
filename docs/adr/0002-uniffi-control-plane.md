# 0002. UniFFI for the Swift ↔ Rust control plane only

Date: 2026-11-13 · Status: accepted

## Context

The Swift UI needs to start/stop the engine, toggle modules, set parameters and read
telemetry. None of that is real-time.

## Decision

Expose a small, typed control API from `hearo-ffi` with UniFFI. The UI polls snapshots (~60 Hz
while visible, never when hidden). UniFFI calls never happen on the audio thread; the engine
moves data between threads with atomics, `rtrb` and `triple_buffer`.

## Alternatives considered

- **Hand-written C ABI + bridging header** — more unsafe code, no generated Swift types.
- **swift-bridge** — smaller community; UniFFI is used at scale (Mozilla) and versioned.

## Consequences (incl. how we'd know it was wrong)

- Exact UniFFI version must match between the Rust crate and the bindgen used for Swift.
- Wrong if: polling snapshots through UniFFI costs > 0.5% CPU at 60 Hz, or generated code fights
  Swift 6 strict concurrency badly enough to need large unsafe wrappers.
