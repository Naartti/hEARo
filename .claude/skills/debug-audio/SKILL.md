---
name: debug-audio
description: Playbook for debugging hEARo live audio - no sound, silence, glitches, wrong apps captured, permission problems, device switches, latency. Use when the live engine misbehaves.
---

# Debugging live audio

1. **Which processes play audio?** `just cli procs --watch` (PID, bundle ID, running output).
   Electron/Chromium/WebKit play from helper processes.
2. **No callbacks / "idle"?** Normal while nothing plays: tap-backed aggregates only run while
   a tapped app produces audio (ADR 0010). Play something, then run
   `cargo run -p hearo-engine --example tap_diagnose` — it prints the TCC permission state and
   whether plain/aggregate/tap IO actually runs. Check
   System Settings → Privacy & Security → Screen & System Audio Recording. Ad-hoc builds lose
   the grant on every rebuild — `just doctor` must show a stable signing identity.
3. **Logs:** `log stream --style compact --predicate 'subsystem == "io.github.naartti.hearo"'`.
4. **Silence / zero buffers after a while?** The app's supervisor (ADR 0014) rebuilds tap +
   aggregate when tapped apps report running output (`just cli procs`) but the IOProc gets no
   callbacks for 2 s, and gives up after two tries (popover shows the problem; audio plays
   unprocessed). Reproduce the policy without hardware in `hearo_engine::supervisor::tests`
   (`FakeBackend`: `Flow::Dead`/`Silent`); on hardware `just test-hw` (`hw_watchdog`) and
   `HearoEngine::counters()` (watchdog rebuilds, give-ups, wake rebuilds). If an app plays but
   `procs` shows it *not* running output, the watchdog can't see the stall — note it in ADR 0014.
   `hearo run` has no watchdog (it only follows the output).
5. **Device switch / AirPods sleep / Mac wake:** confirm the rebuild fired (device changes
   debounced 250 ms; wake detected within ~0.5 s from the wall-clock jump).
6. **Level drop on multichannel interfaces:** compare tap vs device level; compensation status.
7. **Glitches:** check underrun/overrun counters, then `just bench`; profile with Instruments
   (Time Profiler + Audio System Trace) if Xcode is installed.
8. **Latency:** `run --stats` shows hEARo-added latency separately from device latency.
9. **Fail-safe:** `kill -9 <pid>` must restore normal audio within 1 s (MutedWhenTapped).
