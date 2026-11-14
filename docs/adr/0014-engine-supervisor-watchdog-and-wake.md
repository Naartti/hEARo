# 0014. Engine supervisor: watchdog and sleep/wake recovery in Rust

Date: 2026-11-14 · Status: accepted

## Context

The tap is `MutedWhenTapped`: while our aggregate holds it, the apps' own output is muted. If
the IOProc stops getting callbacks (device hiccup, `coreaudiod` restart, sleep) the user hears
**silence**, the worst failure hEARo can have. But zero callbacks is also the normal *idle*
state — a tap-backed aggregate only runs while a tapped app plays (ADR 0010) — so "no
callbacks" alone can't trigger anything. Core Audio process objects expose
`kAudioProcessPropertyIsRunningOutput` (in `objc2-core-audio` 0.3.2, already used by
`hearo procs`), which tells us whether a tapped app *should* be producing callbacks.
What the tap/aggregate do across system sleep isn't documented and hasn't been observed yet.
The output-follow logic lived in `hearo-ffi` and could only be tested on hardware.

## Decision

We put all recovery policy in `hearo_engine::supervisor::Supervisor`, generic over an
`AudioBackend` trait (start = build tap + aggregate + IOProc, output changed?, controls,
callback counters, "are tapped apps playing?"); `LiveBackend` is the Core Audio one and
`FakeBackend` scripts the world for unit tests with an injected clock. One control thread
(`hearo-supervisor` in `hearo-ffi`) ticks it every 500 ms and on settled device notifications.
Policy: rebuild on output/rate change; retry failed starts with backoff 3 s → 30 s; **watchdog**:
if tapped processes report running output but the IOProc got no callbacks for 2 s, rebuild
(then after 2 s, then give up after 4 s more: tear down so macOS plays audio unprocessed and
report it in `EngineStatus.problem` until the user toggles hEARo, the output changes or the
Mac wakes); if callbacks carry only digital silence, rebuild with backoff 2 s → 60 s but never
give up (a silent call is legitimate, and rebuilding silence is inaudible). The watchdog doesn't
judge while TCC is `NotDetermined` (the permission prompt may be up). **Sleep** is detected in
Rust without new dependencies: `Instant` (`CLOCK_UPTIME_RAW`) stops during sleep, the wall
clock doesn't; a ≥ 3 s divergence between ticks (or a ≥ 30 s gap) means we slept, and we
rebuild tap + aggregate unconditionally.

## Alternatives considered

- **Swift `NSWorkspace.didWakeNotification` → FFI `restart()`** — works, but splits the logic
  across languages and the CLI wouldn't get it; kept as a fallback if clock detection misses.
- **IOKit `IORegisterForSystemPower` in Rust** — exact, but needs a run loop thread and a new
  binding crate (`objc2-io-kit`) for something the clock comparison already gives us.
- **Rebuild only if the engine looks dead after wake** — the watchdog does that anyway; an
  unconditional rebuild costs ~0.3 s of unprocessed (never silent) audio at a moment nobody
  listens, and covers states we can't detect.
- **Treat silent buffers like missing callbacks (give up)** — would switch hEARo off during
  every quiet call.

## Consequences (incl. how we'd know it was wrong)

- All recovery paths are unit-tested (`supervisor::tests`) without hardware; the
  Core Audio side has `just test-hw` and the sleep/wake item in the hardware checklist.
- Wrong if: users report silence after wake or mid-session that lasts > ~3 s (the watchdog
  missed it: e.g. `IsRunningOutput` false while apps play into a dead tap); `watchdog_rebuilds`
  climbing while audio is fine (false positives, e.g. apps playing to a non-default device that
  the tap doesn't run for); or no rebuild after wake (clock heuristic wrong → add the Swift
  notification).
