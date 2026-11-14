# 01. Core Audio taps: how hEARo replaces system audio

## The pieces

| Thing | What it is | Music-studio analogy |
|---|---|---|
| **HAL** (Hardware Abstraction Layer) | The layer of Core Audio that talks to devices | Your audio interface's driver |
| **Device** | Something with input and/or output streams (speakers, AirPods, an interface) | The interface itself |
| **Process object** | Core Audio's record of an app that plays audio (`hearo procs`) | A track in your session |
| **Process tap** | A copy of what processes play, captured *before* it's mixed into a device | A pre-fader send on the master bus |
| **Aggregate device** | A virtual device built from other devices (and taps) that share a clock | Combining two interfaces into one in Audio MIDI Setup |
| **IOProc** | Our callback, run on Core Audio's real-time thread every buffer | The plugin's `process()` |

## The topology

```mermaid
flowchart LR
  subgraph apps[Apps]
    T[Teams] & Y[Browser] & P[Podcasts]
  end
  apps -- "output (muted while tapped)" --> TAP[Process tap<br/>global, minus hEARo + music apps]
  subgraph AGG[Private aggregate device 'hEARo']
    direction TB
    TAP2[tap stream = input] --- OUT[output device = main sub-device<br/>(clock master)]
  end
  TAP --> TAP2
  TAP2 -- IOProc: chain --> OUT
  OUT --> SPK((Speakers / AirPods))
```

1. **Tap everything except a few processes.** `CATapDescription` with
   `initStereoGlobalTapButExcludeProcesses` mixes every app's output to stereo, except hEARo
   itself (otherwise we'd hear ourselves forever — a feedback loop) and the excluded music
   apps/DAWs. Electron apps like Slack and Teams play from *helper* processes, so "everything
   except" is far more robust than "only these PIDs". (`crates/hearo-engine/src/ca/tap.rs`)
2. **Mute the originals only while we listen.** `MutedWhenTapped` silences the tapped apps'
   normal output *only while someone reads the tap*. If hEARo crashes or is killed, nobody reads
   the tap and macOS plays the apps normally again — the core of the fail-safe.
3. **One aggregate, one clock.** A *private* aggregate device (it vanishes when our process
   exits) has the current output device as its **main sub-device** (the clock master) and our
   tap in its tap list (with drift compensation). Input (tap) and output (speakers) now tick
   together, so a single IOProc reads the tap and writes the speakers — no resampling, no drift.
   (`crates/hearo-engine/src/ca/aggregate.rs`)
4. **Only open the tap.** If the output device also has inputs (AirPods have a mic), the
   aggregate exposes them too. Opening the AirPods mic would switch them to the low-quality
   call profile, so the engine turns off every input stream except the tap for its IOProc
   (`kAudioDevicePropertyIOProcStreamUsage`). (`LiveEngine::use_only_tap_input`)
5. **Process.** Each IOProc cycle de-interleaves the tap into planar L/R, runs the `Chain`, and
   writes L/R to the output buffers. Panics are caught and turn into plain passthrough.
   (`crates/hearo-engine/src/rt.rs`)

## Idle is normal; permission is checked separately

A tap-backed aggregate is lazy: **its IOProc only runs while some tapped app is actually
playing audio.** Start hEARo with nothing playing and `AudioDeviceStart` succeeds, but no
callbacks arrive until you press play — then they start within one buffer. hEARo shows this as
*idle*. (We first misread idle as a permission problem — see the history in
[ADR 0010](../adr/0010-engine-on-objc2-core-audio.md).)

macOS asks for **System Audio Recording** (Privacy & Security → Screen & System Audio
Recording) the first time a tap delivers audio. The permission belongs to the *responsible
app*: Terminal/iTerm/VS Code when you run `hearo run`, *hEARo* when the app runs the
engine. There's no public API to check it, so hEARo asks TCC's private `TCCAccessPreflight`
(`crates/hearo-engine/src/ca/permission.rs`) and refuses to start only if it says *denied*.

To check yourself (start some audio first):

```sh
cargo run -p hearo-engine --example tap_diagnose
# System Audio Recording permission: Authorized
# aggregate + bogus tap:   callbacks = 59   ← the device works
# aggregate + real tap:    callbacks = 55   ← the tap runs (0 if nothing is playing)
```

Permissions are also tied to the app's code signature — see deep dive 13 and
[SIGNING.md](../dev/SIGNING.md) for why dev builds use a stable signing identity.

## When the tap goes quiet: the watchdog

Because the tap mutes the apps while we hold it, a *dead* IOProc (no callbacks) would mean
the user hears nothing. But "no callbacks" is also plain idle. The tie-breaker is each
process object's `kAudioProcessPropertyIsRunningOutput` — the same flag `hearo procs` shows:
if some tapped app (not hEARo, not an excluded app) is running output and our IOProc has had
no callbacks for 2 s, something is wrong and the supervisor rebuilds tap + aggregate. If that
doesn't help twice, it tears everything down — macOS then plays the apps unprocessed — and
says so in the popover. Callbacks carrying exact digital zeros while apps play are rebuilt
more patiently and never cause a give-up: a call where nobody talks looks exactly like that.

After **system sleep** hEARo rebuilds tap + aggregate as soon as it notices the wake (Rust's
`Instant` doesn't advance during sleep while the wall clock does). What Core Audio does to a
tap across sleep is still to be observed on hardware — see the checklist. Details:
[ADR 0014](../adr/0014-engine-supervisor-watchdog-and-wake.md).

## Latency

The status line shows two numbers:

- **added by hEARo** = one IO buffer (e.g. 512 frames ≈ 11.6 ms at 44.1 kHz) + the chain's
  lookahead. This is what our budget (≤ 30 ms) is about.
- **device** = the output device's own latency + safety offset (Bluetooth adds a lot here; it
  exists with or without hEARo).

## Experiments to try

```sh
just cli procs --watch          # start a YouTube video and watch its helper process appear
just cli run                    # passthrough; type b + Enter to A/B — should be identical
just cli run --modules gain --param gain.gain=-12    # toggle 1 + Enter: a smooth 10 ms fade
pkill -9 hearo                  # (from another terminal) audio returns by itself
```
