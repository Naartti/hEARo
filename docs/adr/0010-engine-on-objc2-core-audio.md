# 0010. Live engine built directly on objc2-core-audio

Date: 2026-11-13 · Status: accepted

## Context

HANDOVER §3.1 suggested evaluating the high-level `coreaudio` crate first. Verified versions:
`coreaudio` 0.6.2 has `ProcessTap` (stereo mixdown of *included* processes, or a device stream)
and a tap-only aggregate device. It lacks the two things our topology needs: a **global tap
excluding processes** (`initStereoGlobalTapButExcludeProcesses`) and an **aggregate whose main
sub-device is the output device plus the tap** (one clock, one IOProc). `objc2-core-audio`
0.3.2 exposes all of it (`CATapDescription`, `setBundleIDs`, aggregate keys, IOProcs,
`kAudioDevicePropertyIOProcStreamUsage`, `kAudioDevicePropertyIOThreadOSWorkgroup`).

## Decision

`hearo-engine::ca` wraps `objc2-core-audio` (+ `objc2-foundation` for `CATapDescription` and
the toll-free-bridged description dictionary) in a small safe layer: property get/set,
process list, device info, `Tap`, `Aggregate` (RAII). The IOProc is a C callback with a
pointer to pre-allocated `RtState`; panics are caught and fall back to passthrough.

Verified behaviour on macOS 26.6 (M5 MacBook Pro), recorded because it surprised us:

- **A tap-backed aggregate only runs its IOProc while some tapped process is playing audio.**
  With nothing playing, `AudioDeviceStart` returns `noErr` and the IOProc simply isn't called;
  it starts within one buffer once an app plays (and stops again later). Zero callbacks is
  therefore the normal *idle* state, not an error — the CLI shows "idle — waiting for an app to
  play audio" and the watchdog (M2) must not treat idle as a stall. (An early version of this
  ADR misread idle as "missing permission"; the TCC preflight below disproved that.)
- The stereo-mixdown tap runs at the system mix rate (48 kHz) and works inside a 44.1 kHz
  aggregate thanks to tap drift compensation (the aggregate resamples). `Tap::device_excluding`
  (tap a device stream at the device's rate/channels) exists for the multichannel-attenuation
  work (§3.3) but isn't needed for rate matching.
- Permission is checked with the private `TCCAccessPreflight("kTCCServiceAudioCapture")` (as
  AudioCap does), loaded via `dlopen` so a missing symbol degrades to `Unknown`. `Denied` stops
  start-up with a clear error; `NotDetermined` proceeds and macOS prompts on first audio.
- Non-tap input streams of the aggregate (e.g. an AirPods microphone) are switched off for our
  IOProc via `kAudioDevicePropertyIOProcStreamUsage`, so we never open a mic (which would force
  the Bluetooth call profile).
- Diagnose with `cargo run -p hearo-engine --example tap_diagnose` while audio plays.

## Alternatives considered

- **`coreaudio` 0.6.2** — missing exclusion taps and output+tap aggregates.
- **Swift for the HAL layer** — contradicts ADR 0001.

## Consequences (incl. how we'd know it was wrong)

- ~400 lines of `unsafe`-wrapping code we own; covered by hardware-reading tests (device info,
  process list) and the CLI. Wrong if: objc2 bindings lag new macOS tap APIs we need — then
  hand-declare the missing selectors.
