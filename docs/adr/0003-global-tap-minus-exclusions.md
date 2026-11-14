# 0003. Global tap minus exclusions as the default capture mode

Date: 2026-11-13 · Status: accepted

## Context

Electron/Chromium apps and Safari play audio from helper processes, so tapping a main PID often
captures nothing. `CATapDescription` supports a stereo-mixdown global tap that excludes a list
of processes; on macOS 26, bundle IDs can be used.

## Decision

Default to a global tap excluding hEARo itself (always) and a user-editable list of music apps
and DAWs. Per-app include mode groups helper processes by bundle-ID prefix and refreshes on
`kAudioHardwarePropertyProcessObjectList` changes. Mute behaviour: `MutedWhenTapped`.

## Alternatives considered

- **Include-list per app by PID** — misses helpers, needs constant re-tapping.
- **Virtual audio driver** — explicitly a non-goal (no drivers/HAL plugins).

## Consequences (incl. how we'd know it was wrong)

- Everything not excluded is processed, including system sounds (acceptable; DSP is transparent
  when nothing triggers).
- Wrong if: global taps add measurably more latency/CPU than per-app taps, or exclusions by
  bundle ID fail for common apps (verify with `hearo procs --watch` in M2).
