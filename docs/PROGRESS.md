# Progress

The resume point for any session. Tick tasks as they land; add a one-line note per task.

**Current milestone:** M4 — DSP modules (M1 done; M2/M3 wait for the
signing identity, H1).
**Now:** de-smack listening test (human). **Done first (re-prioritised by H2 feedback):** lip-smack removal — a pause-aware mouth-click /
lip-smack module (M4 de-click, extended to 5–30 ms smacks), tuned on synthetic smacks + the
human's captured snapshots (H4 early). Then M2 device-change rebuild + watchdog + FakeBackend,
M3 bridge, de-hum rework.

Terminology (for docs/UI): *filled pause* = "uhm/öh" filler; *lip smack* = lips unsticking after a pause (sharp, bright, 5–30 ms);
*mouth/tongue click* = tiny tick < 5 ms; *plosive* = low "p/b" air thump; *wet mouth noise* =
saliva crackle overlapping words.

**Formerly WIP (de-hum) — committed 2026-11-13.** Old note: `crates/hearo-dsp/src/{filters.rs,modules/dehum.rs}`,
`crates/hearo-dsp/tests/golden_dehum.rs`, `modules/mod.rs` registration, `lib.rs` (`pub mod
filters`), `hearo-testkit` metrics `energy_excluding_db` + `tone_amplitudes`. 5/7 golden tests
pass (hum −62…−69 dB on exact 50/60 Hz, 0.04 dB outside notches, clean speech untouched);
the 2 new review tests fail as expected.

## M0 — Bootstrap

- [x] Toolchains pinned in `.mise.toml` (Rust 1.100.0, just 1.59.0, Python 3.12.16, uv 0.13.8,
      node 22.23.5, lefthook 2.2.2, nextest 0.9.151, cargo-deny 0.20.2, machete 0.9.2,
      xcodegen 2.46.0, lychee 0.24.2, markdownlint-cli2 0.24.1); Brewfile for xcbeautify, create-dmg.
- [x] Cargo workspace, 6 crates, exact dependency pins, `rust-toolchain.toml`, `rustfmt.toml`.
- [x] `justfile` + `scripts/` (bootstrap, doctor, check, ci, golden, sign, build-app, ...).
- [x] Local CI instead of GitHub Actions (ADR 0009); lefthook hooks.
- [x] `deny.toml` (permissive licenses only) — passes.
- [x] `.vscode/`, `AGENTS.md`, `CLAUDE.md`, `.claude/` (settings, 4 hooks, 6 subagents, 5 skills).
- [x] Docs skeleton, ADRs 0001–0009, HANDOVER copied to docs/.
- [x] Minimal app: SwiftPM `HearoKit` + `@main` MenuBarExtra shell with FakeEngine, builds with
      Command Line Tools; `project.yml` validates with XcodeGen.
- [x] `ml/` uv project (ruff, pytest) with a smoke test.
- [x] `just ci` green (Xcode installed; Swift tests run). Fresh-clone run at milestone review.
- [x] H1 raised (OPEN_QUESTIONS.md); Xcode done; signing identity + icon choice still open.
- [x] Fresh-clone `just ci` green (76 s cold). Tagged `m0-done`.

## M1 — DSP foundation (offline)

- [x] `AudioBlock` (planar, zero-copy sub-blocks), `Module` trait, `ModuleId`, `ParamDesc`.
- [x] `Chain`: constant latency compensation, 10 ms raised-cosine toggles with warm-up for
      lookahead modules, bit-exact global bypass, NaN/Inf guard, chunking to `max_block`.
- [x] `TelemetryWriter`: fixed-capacity events + per-module meters, drop counting.
- [x] Analysis: streaming `StftAnalyzer` (Hann, normalised to dBFS) + `LogBands` (1/N octave).
- [x] `hearo-testkit`: SplitMix64 RNG, speech-like/music-like/pink/sweep/hum/click/smack/
      sibilance/plosive/pluck/drum generators, metrics, WAV I/O, counting allocator.
- [x] CLI: `gen-signals`, `process --report`, `inspect` + end-to-end test.
- [x] Property tests: all-off and bypass bit-exact, block-size invariance, finite output.
- [x] RT-safety: zero allocations in chain + analysis; 10-min randomized stress (< 1 s wall).
- [x] Benchmarks (divan) + CPU budget test (chain p99 0.008% of real time with utility modules).
- [x] Deep dive 02 (real-time audio in Rust).
- [x] Milestone review run (rt-safety-reviewer, test-gap-finder, docs-keeper) 2026-11-13.
- [x] Fix review findings (punch list below) + tag `m1-done` (2026-11-13; `just ci` green).

### M1 review punch list

Fixes (RT/correctness):

- [x] Chain: sanitize non-finite *input* to 0 before it enters dry/bypass delay lines (today a
      NaN lingers up to `latency` samples, re-faulting and resetting modules every chunk);
      on a module-generated fault reset only the offending module.
- [x] Gain: ramp over a fixed ~10 ms (sample-count based, carried across blocks), not "one
      block" — current behaviour breaks block-size invariance on parameter changes.
- [x] Denormals: FTZ guard (aarch64 FPCR.FZ) at the top of `Chain::process` before IIR modules land.
- [x] Chain: `module/slot_state/is_enabled` return `Option` (no panics on bad index); assert
      `channels <= MAX_CHANNELS` in `prepare`.
- [x] `AudioBlock::copy_from`/`channel` and `MultiDelay::process`: debug_asserts on shape.
- [x] `LogBands::with_enbw`: clamp to > 0. `Wav::from_planar` with 0 frames panics.
- [x] Fix stale ADR link `docs/adr/0009-local-ci.md` in justfile + scripts/ci.sh.

Tests:

- [x] Bit-exact after toggling *off mid-stream* and bypass mid-stream (fade paths), proptest.
- [x] Enabling a lookahead module mid-stream settles aligned and scaled (warm-up output).
- [x] Block-size invariance with a parameter change at an aligned sample index.
- [x] NaN fault recovery: faults stop growing after `latency + block`; tail matches clean ref.
- [x] Telemetry `block_start` per sub-chunk; event push + overflow under `count_allocs`.
- [x] CLI: extract `render()`, test latency flush/trim with a lookahead module; stereo bypass
      bit-exact; parse report JSON and assert values; error paths.
- [x] WAV 16/24-bit read scaling; `LogBands` low-frequency fallback level; STFT chunking invariance + reset.
- [x] RT test: toggle every slot off *and* on; Lookahead at -6 dB in stress; check all channels.
- [ ] Every `ModuleId` except Utility is in `modules::KEYS` (once M4 modules exist).
- [x] `just ci`: add `cargo bench --no-run` so benches always compile.

Docs:

- [x] Deep dive 02: audio workgroups section + a mermaid diagram.
- [x] ARCHITECTURE: global bypass keeps modules running, disabled slots don't; faults also
      count unprepared/mismatch passthrough.
- [x] Unify `just check` timing in AGENTS.md / LOCAL_CI.md (warm ≈ 4 s today); STRATEGY rows.
- [x] `just run` log subsystem has no logger until M3 — note it.

**M1 metrics (2026-11-13):** nextest 43 pass (+1 timing test via `just bench`), Swift 6 pass,
pytest 1 pass; chain p99 ≈ 0.9 µs per 512-frame stereo block (0.008% RT, utility modules only);
warm `just check` ≈ 4 s, cold `just ci` 76 s.

## M2 — Live engine (passthrough)

- [x] Core Audio layer on objc2-core-audio (ADR 0010): global tap excluding self + music apps
      (`MutedWhenTapped`), private aggregate (output = main sub-device, tap with drift comp),
      IOProc running the chain, only the tap input stream enabled, panic → passthrough.
- [x] `hearo procs [--watch]`; `hearo run` with live stats, bypass/module toggles from stdin.
- [x] Permission checked via TCC preflight (denied → clear error). Idle (no app playing ⇒ no
      IOProc callbacks) shown as idle, not an error — found on real hardware (ADR 0010).
- [x] H2 listening test passed 2026-11-13 (passthrough identical, smooth gain, kill -9 safe).
- [x] Early M6 capture: lock-free 12 s ring of unprocessed input; `m`/`f` keys in `hearo run`
      save 8 s WAV + JSON sidecar to ~/Library/Application Support/hEARo/snapshots.
- [x] Global `§` hotkey (Carbon, no Accessibility) + notification/terminal confirmation;
      20 s ring so snapshots are a full 8 s; `event_hint_s` in the sidecar.
- [x] `AudioBackend` trait + `FakeBackend`; `hearo_engine::supervisor` owns rebuild/retry/
      watchdog/wake policy (ADR 0014), 21 unit tests with an injected clock; hearo-ffi drives it
      from the `hearo-supervisor` thread (500 ms tick). Retries back off 3→30 s; only a new
      default output retries at once (our own aggregate fires device notifications).
- [x] Zero-buffer watchdog: tapped processes report IsRunningOutput but no callbacks for 2 s →
      rebuild (×2), then give up → passthrough + `EngineStatus.problem`; silent-only buffers
      rebuild with 2→60 s backoff, never give up. Sleep/wake: wall-clock vs `Instant` jump →
      unconditional rebuild. `just test-hw` adds `hw_watchdog`; sleep/wake + false-positive
      checks are human items in HARDWARE_CHECKLIST. `hearo run` has no watchdog yet.
- [x] Rebuild on default-output / sample-rate change, device loss (debounced 250 ms).
      Human report 2026-11-14 (AirPods ignored). `ca::watch` + follower thread in hearo-ffi,
      `hearo run` too; `just test-hw` switches outputs and back. AirPlay/Sonos untested.
- [ ] Multichannel attenuation check; audio workgroup join (if a worker thread is ever added).

## M3 — App shell + bridge

- [x] UniFFI 0.32.2 control plane (`hearo-ffi`): start/stop, bypass, module toggles, status
      record (idle, latency, CPU, apps playing, peaks, faults), snapshots, permission.
- [x] `just ffi` → static lib + generated Swift + XCFramework (incremental, no-op 0.14 s);
      `just check`/`just app` build it automatically.
- [x] App drives the live engine: RustEngine (HearoLive) behind the `Engine` protocol; engine
      starts at launch; 5 Hz polling; status line incl. "Waiting for audio"; bypass button;
      start errors shown in the popover.
- [x] `§` global hotkey in the app (Carbon) → snapshot + menu bar checkmark; accent glow when a
      module removes something (DESIGN icon table). 10 Swift tests.
- [x] MVP test with the app passed (human, 2026-11-13).
- [x] Live waveform in the popover (engine column ring → FFI `waveform(since:)` → Canvas;
      red = removed; 30 Hz polling only while open). Human request 2026-11-13.
- [x] Snapshots across Macs: per-install device id in every sidecar; export/import/merge
      (never overwrites, dedupes incl. earlier renames) via `hearo dataset` and the popover ⋯ menu.
- [x] Launch at login (SMAppService) in the ⋯ menu; `just release` → signed DMG
      (docs/user/INSTALL.md).
- [ ] Sources list, onboarding state, snapshot tests (swift-snapshot-testing), launch at login.

## M4 — DSP modules

- [x] De-hum — reworked after dsp-reviewer (2026-11-13): mains-frequency tracking (complex
      demodulation of the cleanest harmonic on mid *or* side, τ ≈ 2 s), ≥ 2 of 3 harmonics must
      be tonal (held bass notes don't engage), detection on ↓32 decimated mid/side (out-of-phase
      hum detected), per-harmonic smoothed depths, stepped Mains param, f64 power. Golden: hum
      −40…−44 dB exact, −35/−30 dB at ±0.1 Hz drift (thin at +0.1: tracker converging),
      −42 dB out-of-phase stereo; clean speech octaves < 0.1 dB; DC removed. 7.4 µs idle.
  - [ ] Review suggestions left: per-harmonic depth telemetry, real in-band reduction in GR,
        AM hum test, bench with notches engaged, deep dive 04 section.
- [x] **De-smack (pause-aware)** — human's top priority. 8 golden tests: recall 0.93–0.95,
      smacks −13…−17 dB, 0 FP/min speech, ≤1 FP/min music, stop consonants 2–4 % ducked,
      smooth at block sizes 1–2048, linked stereo. Real snapshots: 14/19 clips. 40 ms lookahead +
      128-frame IO buffer ≈ 43 ms (ADR 0011). Deep dive 06. Reviews: dsp + rt (all fixed).
  - [ ] Human listening test in the app (H3) — then tune on "false hit"/"missed" snapshots.
  - [x] Loud smack with a slow tail missed (human demo recording, 2026-11-14): explain why.
        Cause: its tail took 120 ms to settle, past the 60 ms limit. A fix was built, then
        rejected in DSP review (ducks word-final/weak-aspiration stops; deep dive 06). Docs only;
        real fix needs more lookahead (H3) or the ML stage.
  - [ ] Stop-consonant test blind spot: measure per stop over burst + aspiration (released
        provisional ducks report no event); add weak-, decaying-aspiration and utterance-final
        stop variants (DSP review 2026-11-14; HEAD already ducks some weak-aspiration stops).
  - [x] Telemetry under bypass/warm-up (RT review 2026-11-14): trigger counters (popover count,
        `.removing` glow) still count while globally bypassed; a slot warming up reports events
        for audio it didn't remove (could auto-save a spurious clip). Count published events in
        `RtState`; truncate events pushed by a `WarmingUp` slot.
        Acceptance: under global bypass the trigger count, glow and reduction stand still; no
        event is reported for audio a slot didn't remove. Done (2026-11-15): `RtState` publishes
        trigger deltas only when not bypassed (reduction reads 0 then). Decision: the chain
        filters by *when the audio leaves the slot*, not by state — a lookahead detector's
        warm-up events are for audio leaving after warm-up (kept; de-smack in fact reports none
        in its first ≈ 45 ms), while fading out it still detects audio that leaves after the
        fade (measured: de-smack reported 9/10 smacks disabled 30 ms in, all 0 dB removed).
        Both states now drop and uncount such events (`TelemetryWriter::retain_events_since`).
        Review fixes (same branch): warm-up's last chunk is all dry, so its events go too; a
        block whose output was replaced after a fault reports no events.
  - [ ] Event timing at toggle boundaries (follow-ups from the item above and its RT + general
        reviews, 2026-11-15). All need events held until their audio has played, then judged
        by the slot/bypass state at that time:
        - events reported while `On` for audio still in the lookahead at disable (≤ 40 ms for
          de-smack), or during a warm-up that is then cancelled, are already published;
        - re-enabling during a fade-out keeps that audio processed, but its events were dropped;
        - global bypass is judged by the target at publish time, not when the audio plays (≤
          chain latency early/late; the 10 ms bypass fade counts as fully bypassed);
        - events past the 256-slot buffer can't be uncounted (documented on
          `retain_events_since`); triggers also count events the full ring drops.
        Rejected: caching `Slot::latency` duplicates `latency_samples()` — latency is fixed
        between `prepare`s (the dry delay is sized from it too), so the cache is the right
        source for the audio-thread filter.
  - [ ] Auto-save follow-ups (reviews 2026-11-14): surface `skipped_by_cap` and dropped events in
        status; snapshot/preview tests for the new visual states; clip cutting assumes the
        chain's clock never stalls (it doesn't advance on its fault path).
  - [x] **Auto-catch false positives (human, 2026-11-15):** "lots of false positives on the
        auto-caught de-smacks" — 92 of 108 reviewed catches rejected. Acceptance: measure on the
        reviewed snapshots, cut false positives substantially without losing real smacks, keep
        every golden test. Done: `hearo dataset eval` (scores against all reviewed clips;
        reproduces 85/92 live rejections offline) and a salience gate — ≤ *below speech*
        (`below_speech`, new param, 22 dB) under the speech level and ≥ 10 % of its energy
        above 1.5 kHz. 111 reviewed clips: FPs 132 → 81, rejected re-caught 90 → 50, the same
        37 / 56 regions found (catch-by-catch: the gate never adds a catch). Default first 18 dB;
        raised to 22 after the human's bar "fewer FPs and at least as many smacks" (18 lost 2).
        Deep dive 06 tuning log. Reviews: rt (clean), dsp + code review (fixed except below).
    - [ ] Human: listen; if still too eager, try *Below speech* 18 dB (66 FPs, 35 found) or
          15 dB (53, 35) in Settings. Then the 19 missed regions and the FPs left
          (`just cli dataset eval -v`).
    - [ ] DSP review follow-ups: hold the speech tracker while a burst is open (a loud smack
          lifts the reference ≈ 3 dB for the next one in a cluster); a dull burst that fails the
          gate still keeps the detector busy ≤ 20 ms.
    - [ ] Auto-save sidecars list every slot in `modules`, not which were enabled or their live
          params, so `dataset eval` runs upstream modules at defaults: record both.
  - [ ] Investigate the 11 `missed` snapshots captured 2026-11-14 afternoon (all 0 dB reduced
        — a different cause).
  - [ ] Review suggestions left: smack 0–5 ms before a word, double clicks, digital-silence
        pauses, HF-only floor, upstream-latency-aware event timestamps.
- [x] **De-ess** (split-band dynamic bell + onset gate, 0 lookahead). 12 golden tests at
      44.1/48/96 kHz: 120 ms "s" over a rich vowel (harmonics to 10 kHz) peak −8.2…−9.9 dB
      (2 ms RMS) / −6.8…−9.4 dB (sample), energy −7.5…−8.5 dB, vowel 0.05 dB; fast-onset "s"
      (10 ms rise) energy −7.8…−8.3 dB, peak −6.0…−8.3 dB; snap train (6–8 kHz, −6 dBFS, 4/s)
      0 events; music_like 0–1 events/min, 5–9 kHz ≤ 0.005 dB; speech_like 0 events; event
      length now ends on the target gain (120 ms "s" → 79–88 ms, was 220 ms); bit-identical
      for block sizes 1–2048. `module_alone/deess` ≈ 9.3 µs per 512-frame stereo block.
      Deep dive 05.
  - [x] **Fixed (was the overnight known issue):** fired on drum snaps (~240/min on
        music_like, −7 dB in 5–9 kHz; 104/min on speech_like at 96 kHz). Onset gate: the cut is
        armed only if the 2 ms band envelope climbs 3 dB above its peak in the first 4 ms
        after crossing the knee (snaps/pulses/plucks peak at once, an "s" keeps building).
        Tried and rejected: dominance gate, decay/sustain gate, long-term relative threshold
        (deep dive 05). `speech_like`'s glottal tilt is now sample-rate independent (2.2 kHz
        one-poles; 48 kHz unchanged) — side effect: de-smack 96 kHz recall 0.95 → 0.90
        (threshold 0.90, at the edge), de-hum 96 kHz −44.4 → −41.4 dB.
  - [ ] Not met: fast-onset "s" first peak is ≥ 6 dB only in the median (20 seeds: worst
        −4.4 dB 2 ms RMS / −3.5 dB sample); the test asserts ≥ 3 dB peak + ≥ 6 dB energy. Needs
        lookahead (ADR 0012 shared lookahead) to fix.
  - [ ] Known limits: an "s" must rise 3 dB above what's already in the band; affricates
        ("ts", "ch") with a loud burst aren't armed. Adaptive centre; dsp-reviewer re-check;
        human listening test (H3).
- [x] **De-plosive** (dynamic low cut, 0 latency) — 13 golden tests: pops −13.2…−13.6 dB LF
      (≥ 10) at 44.1/48/96 kHz, 100 % found; after silence −17 dB; speech, music + kicks, bass
      line, low voice, 90 Hz drone: 0 events, bit-exact; block sizes 1–2048 identical.
      `module_alone/deplosive` ≈ 8.4 µs per 512-frame stereo block. Deep dive 04.
  - [ ] Known limit: pops quieter than the voice (−22…−16 dBFS) only −5 dB, 85 % found.
  - [ ] dsp-reviewer (meets spec) follow-ups: settle/ramp the HP on cutoff changes; a pop
        generator with −12 dB content at 150–400 Hz; assert bit-exact output when no events.
  - [ ] dsp-reviewer / rt-safety-reviewer pass; human listening test (H3).
- [x] **Safety limiter** (true-peak, 4× polyphase, ≤ 0.5 ms lookahead, always last): no output
      sample or 4× true peak above −1 dBTP at 44.1/48/96 kHz (worst −1.095 dBTP, pink noise);
      speech below the ceiling bit-exact; block sizes 1–2048 identical; property test. ≈ 25 µs
      per 512-frame stereo block. Deep dive 05.
  - [ ] Known trade-off: hot content near Nyquist (≥ 0.42·fs) over-limited (−6.6 dBTP); ceiling
        margin ≈ 0.1 dB is measured, not proven. Reviews pending.
- [x] **De-click (LPC + AR interpolation)** — 2.5 ms lookahead (budget after de-smack). Golden:
      recall 1.0 (44.1/48/96 kHz), 0 FP/min on speech, music (drums, plucks) and a chord,
      bit-identical across block sizes 1–2048. Residual click energy −14 dB and repaired-region
      SNR 23–25 dB at 48 kHz: **below the §7.3 targets (−20 / 25 dB)**, limited by ≤ 0.5 ms of
      post-context; documented in the test and deep dive 06. Bench: 36 µs per 512-frame stereo
      block (+≈ 100 µs per repair, capped at 20 repairs/s).
  - [ ] Listening test on real mouth clicks; decide whether more lookahead is worth it.
  - [ ] 96 kHz: < 0.7 ms clicks in speech only −9 dB (not understood).
  - [x] rt-safety review: worst callback 766 µs vs 667 µs at 96 kHz (64 frames) → AR order
        capped at its 48 kHz value, ≤ 1 repair per callback, IO buffer scaled with the sample
        rate (64 @ 48 kHz, 128 @ 96 kHz); dense clicks now 413/1333 µs (48 kHz), 541/1333 µs
        (96 kHz), enforced by `budgets.rs`. Zero-channel / non-finite input guarded; repair path
        allocation-counted. dsp-reviewer still to run.
- All M4 modules built → **H3** listening test.

## Backlog (human ideas, prioritised)

- [x] **Rust/FFI foundation (2026-11-14, ADR 0015):** ParamDesc step/choices + snap, primary
      "amount" param per module (de-hum new `amount`, de-plosive max_cut, de-click sensitivity,
      de-smack range (now 0..48), de-ess range; limiter always on); module_catalog / set_param /
      settings / apply_settings / settings_to_json|from_json (tolerant), mirrored into the
      supervisor so they survive rebuilds; live waveform columns carry the processed peak + per-
      module GR (0.25 dB); render_clip over a snapshot (shared `hearo_dsp::offline::render` with
      `hearo process`; bypass bit-exact; 8 s ≈ 50 ms). Swift glue + FakeEngine fakes.
- [x] **Module UI (human request, 2026-11-14):** per-module colours (`ModuleStyle`, Okabe–Ito
      based, ≥ 3:1 contrast, checked for colour-vision deficiencies; the popover list is the
      legend); popover amount slider per module + Advanced settings window (every param,
      per-module / global reset; ⋯ and right-click "Advanced settings…"); settings persisted to
      `~/Library/Application Support/hEARo/settings.json` (atomic, debounced, tolerant, applied at
      start); live waveform with outside-in per-module bands (depth = share of amplitude removed,
      split by dB in chain order, core = measured output); review-window processing inspector
      (⌥⌘I): trial settings rendered offline via `render_clip`, **B** = A/B bypass at the same
      playhead (loops kept), bands + event ticks on the review waveform, "Apply these to live" /
      "Use my live settings". 114 Swift tests.
  - [ ] Open: image snapshot tests; inspector slider rows don't fill the panel width; no module
        overlay on the spectrogram lane; A/B skips ≈ 10–20 ms of output latency per switch.
- [x] **One hEARo window with tabs (human request, 2026-11-14):** Review snapshots and
      Advanced settings merged into one native window with toolbar tabs, and the popover's
      controls as a tab so they stay open for validation/testing. Acceptance: (a) one window,
      native-looking tabs Controls · Review · Settings (segmented toolbar control, ⌘1–⌘3);
      (b) Controls tab = the popover content, stays open, live waveform runs in both; (c) the
      popover can open it (window button) and every existing entry (Advanced…, ⋯ / right-click
      Review / Advanced) lands on the right tab; (d) no other separate windows remain.
      `MainWindow.swift`, `ControlsView` (was `PopoverView`), menu entry "Open hEARo Window".
  - [ ] Found while verifying (on `main` too): the open popover / Controls tab costs ≈ 40 %
        of a core (app 18 % → 58 % in `top`, processing on) — profile the 30 Hz waveform redraw.
- [x] **Dock icon while the window is open (human request 2026-11-14):** "if it's best practice
      then we should". Acceptance: (a) no Dock icon, not in ⌘Tab, no menu bar while only the
      menu bar icon/popover is in use, and none at launch; (b) opening the hEARo window by any
      route shows the Dock icon, puts hEARo in ⌘Tab and shows its menu bar; (c) closing the
      window removes them again; (d) the menu bar has About hEARo, Settings… ⌘, (Settings tab)
      and View → Controls/Review/Settings ⌘1–⌘3. `MainWindowCommands`,
      `DockPresence`, `AppModel.openMainWindow` / `tabCommand`; DESIGN.md *Dock icon while the
      window is open*. Checked in the app: opening → Foreground with the menus above; closing →
      UIElement, no window.
  - [x] Follow-up: the ⋯ menu's "Advanced settings…" opens the tab the menu bar calls
        "Settings…" — rename it to match (and the popover's "Advanced…" link?). Done
        2026-11-15: the ⋯ / right-click entry and the popover link are both **Settings…** (one
        title, `AppMenu.Item.settings`; the menu entry shows ⌘,), the view is
        `SettingsTabView`; DESIGN.md *One name per place* + *The hEARo window → Naming*.
        Review notes: kept ⌘, on the ⋯ menu inside the window although the app menu has it
        too — both open the Settings tab, and ⌘Q already works the same way; no custom
        VoiceOver label on the "Settings…" link (VoiceOver reads the ellipsis as menus do).
  - Review notes: HANDOVER §13 left as is (the brief stays frozen; DESIGN.md records the
    deviation, like the menu bar icon decision). About/save panels open when the window
    closes may lose their Dock icon with it — accepted, accessory apps show them fine.
- [x] **Spectrogram in the review window (2026-11-14):** log-frequency (30 Hz–20 kHz) magma
      lane under the waveform, sharing viewport/playhead/axis; marking, zoom and regions work on
      both lanes. ⇧⌘S / header toggle (remembered); resolution menu time 512 / balanced 1024 /
      frequency 4096-pt at 48 kHz. Accelerate STFT (`Spectrogram.swift`), off-main cancellable
      cache with deep-zoom renders; 8 s @ 48 kHz ≈ 14 ms release. Editor moved to
      `ReviewTimeline.swift` so the side panel and per-module overlays slot in. 22 tests.
  - [ ] Debt: no snapshot tests for the lane (no snapshot-testing setup in the repo yet).
- [x] **App identity (human, 2026-11-14):** one app "hEARo" (`dev.hearo.app`) for debug and
      release at `~/Applications/hEARo.app` (`scripts/install-app.sh`, one copy per Mac;
      `HEARO_APP_DEST` builds elsewhere). Icon from SVG sources in `app/Resources/Icon/`
      (`scripts/make-icon.swift`, `just icon`, rendered at build time). Info.plist polish, "About
      hEARo" first in the app menu, DMG volume "hEARo" with the icon. ADR 0005 amended. Migration:
      re-grant System Audio Recording once for the new bundle id.
- [x] **Open-source readiness (2026-11-14):** LICENSE-MIT/LICENSE-APACHE (The hEARo
      contributors), licence in ml/pyproject, accurate THIRD_PARTY_NOTICES; public README;
      CONTRIBUTING, CODE_OF_CONDUCT (Covenant 2.1), SECURITY (GitHub private reporting), issue/PR
      templates (no workflows, ADR 0009); CHANGELOG; docs/README.md index. Signing-cert holder
      name removed from OPEN_QUESTIONS.
  - [x] History squashed to one anonymous initial commit (2026-11-14, owner's request): author
        `naartti` with a GitHub noreply address; old history, tags `m0-done`/`m1-done` and
        reflogs removed. `.github/CODEOWNERS` → @naartti.
  - [ ] Owner: check the `dev.hearo` bundle-id prefix. (`repository` set in Cargo.toml and
        inherited by every crate, 2026-11-14.)
  - [ ] Proposals: move HANDOVER/PROGRESS/OPEN_QUESTIONS to docs/project/; crates inherit
        authors/repository once a URL exists; ARCHITECTURE crate table misses hearo-bindgen.
- [ ] **Proper open-source project on GitHub (human request, 2026-11-14):** "looks like and
      behaves like a proper open source project"; PRs are nice; Actions are free for public
      repos. Acceptance: (a) GitHub Actions runs the full gate on every PR and push to `main`
      (ADR 0016 supersedes 0009; the local gate and hooks stay); (b) a `v*` tag builds the app in
      CI and publishes a GitHub Release with a downloadable DMG, so another Mac (or anyone) can
      install without cloning; (c) the README shows the app icon, badges (CI, release, coverage,
      licence, platform) and screenshots; (d) test coverage is measured (`just coverage`, CI);
      (e) the agent workflow ends in a PR instead of a local merge.
      Done on `ci/github-actions-and-releases`: `ci.yml` (just ci + coverage on `macos-26`),
      `release.yml` (ad-hoc signed DMG + sha256 + provenance → GitHub Release; ad-hoc because
      the dev certificate carries the owner's name), Dependabot, release-note categories, SHA
      pinning enforced by `check-files.sh`, `just coverage` (Rust ≈ 86 % regions locally), README
      header/badges/screenshots (WebP, video cropped out), INSTALL.md for downloads, AGENTS.md
      step 8 → PR. Ad-hoc release + DMG built and checked locally (`Signature=adhoc`, no Team ID).
  - Review (code-review, medium): fixed — release gating (tag on `main` + green CI, RC →
    pre-release), read-only build job / tag-only publish job, pin check (quotes, Docker
    digests, `@main # @<sha>` comment trick), signing-identity rule in one `lib.sh` helper; added
    actionlint + shellcheck to `just ci`. Rejected: coverage as a separate job duplicates the
    test run — kept so the gate stays exactly `just ci` and both run in parallel on free
    runners; the tag check's `grep '^version'` copy of build-app.sh — `[workspace.package]` is
    the only such line, and a mismatch fails loudly at the tag check.
  - [ ] Hosted runs blocked: "account is locked due to a billing issue" (owner fixes in GitHub
        billing); then re-run PR #1's checks.
  - [x] CI lost rustfmt/clippy on a mise cache hit (PR #2, 2026-11-14): mise reported
        `rust@1.100.0 already installed` and sets `RUSTUP_TOOLCHAIN`, so rustup ignored
        `rust-toolchain.toml`'s components. Fix: components on the `rust` tool in `.mise.toml`
        (which also changes the mise cache key).
  - [ ] Flaky on the hosted runner: `ReviewInspectorTests.staleRenderResultIsDropped` (saw 2
        processed loads instead of 1 once; passed on re-run). Make it independent of timing.
  - [x] First release: `v0.1.0` (human said go, 2026-11-15): CHANGELOG moved under 0.1.0 on
        `chore/release-v0.1.0`; tag pushed after the PR merged and CI on `main` was green.
  - [ ] Swift coverage (`swift test --enable-code-coverage`) not measured yet.
- [x] **Dependabot routine (human request, 2026-11-15, ADR 0019):** "let Claude Code handle the
      Dependabot PRs autonomously and make sure everything works". `dependency-updates` skill
      (triage → class A merge / B normal workflow / C ignore + note, revert path, report),
      AGENTS.md section, `uv` ecosystem for `ml/`. Owner's manual steps: Dependabot alerts +
      security updates on, `just ci` required in the `main` ruleset. `gh` read/comment/rerun
      permissions added by the owner; `gh pr merge` deliberately left out, so merges prompt.
      The Bash hook turns any auto-allowed `gh` call that may reach another repo, mutate via
      `gh api` or check out a non-Dependabot PR back into a prompt (`gh_scope.py`, 57 cases in
      `scripts/test-hooks.sh`, run by `just check`). Two review rounds; known limit: a
      determined adversary with arbitrary shell can always find a path the prefix rules miss.
  - **Dependency notes** (ignored/deferred updates: what, why, when to retry): none yet.
- [ ] **De-uhm (filled pauses: "uhm", "eh", Swedish "öh")** — low priority (human, 2026-11-13).
      Real time can't remove time, so the module *ducks* fillers (e.g. −12…−20 dB with soft
      fades) rather than cutting them. Needs an ML detector (voiced, speech-like) → M7/M8 after
      de-smack; Swedish "öh" must be in the training data. Annotation class added now so
      snapshots can be labeled for it already. Hotkey labeling: § works for these too.

## M5 — Visualizer · M6 — Labeling · M7 — ML pipeline · M8 — ML in Rust · M9 — Release 1.0

See docs/HANDOVER.md §14; tasks get broken out here when the milestone starts.

- [x] M7 dataset plan (human request 2026-11-14): build it from hotkey snapshots over days of
      normal use instead of a guided recording session; "reviewed" = every event marked, so
      unmarked audio is the negative set; mix clean events from training days; split by day.
      ADR 0017, docs/ml/README.md.
  - [x] Follow-up for M6: make "reviewed = exhaustive" visible in the Review tab (hint text
        near the Reviewed toggle). Done 2026-11-15: caption under the toggle ("Mark every mouth
        noise first" / "Every mouth noise marked"), full explanation as tooltip and VoiceOver
        hint (`ReviewedHint`); DESIGN.md *Review* deviations.
- [x] **Auto-save de-smack catches** (human request 2026-11-14). Done: event ring off the audio
  thread, `autocapture` policy + `save_auto`/expiry, FFI settings + status counters, Settings
  section + orange ear, Review A/X with `rejected` (ADR 0018, DESIGN *Auto-saved de-smack
  catches*, docs/ml/README.md). Needs a human try-out in the app (How to verify in the PR).
  Acceptance:
  - Settings: on/off (default **off**), daily cap (default **250**, adjustable), auto-delete of
    unreviewed auto clips after **31** days (adjustable).
  - When on, every de-smack trigger saves a clip around it (catches close together share one
    clip) with the detection pre-marked as a proposal with start and end; the ear icon flashes
    **orange** (de-smack colour) when it's saved.
  - Review: clearly labelled keyboard shortcuts to accept a detection (verified smack) or mark it
    a false positive (stored as a hard negative), then Return = reviewed + next.
  - Engine path RT-safe (events leave the audio thread through a lock-free ring); tests first.

- [x] M6 (early): snapshot review & annotation window — list, waveform with hint/proposals/
      regions, drag to create/move/resize, zoom/pan, 1–6 classes, Space plays the clip under
      the mouse (selection loops), ↑/↓, Return = reviewed + next; edits merged into the
      sidecar preserving unknown keys. 25 Swift tests (model layer + ReviewModel). Not yet:
      spectrogram, ⌥Space processed A/B, delete snapshot with undo (docs/DESIGN.md).
      Needs a human click-through (gestures, keyboard focus, playback).

## Log

- 2026-11-15 — `codecov.yml`: project status tolerates a 1 % drop (patch 5 %). Human: a docs-only
  PR (#5) failed `codecov/project` at -0.02 %; the default is zero tolerance, which is noise.
- 2026-11-15 — de-flaked `staleRenderResultIsDropped` / `onlyOneRenderRunsAtATime` (failed on a
  hosted runner: a 50 ms sleep overshot a 300 ms fake render). The fake renderer now holds
  chosen calls until the test releases them, so ordering no longer depends on timing.
- 2026-11-14 — agent workflow: "Any change" section in AGENTS.md (intent capture into DESIGN.md,
  feature branch + ff-merge, test-first commits, area reviewers + branch code review, How to
  verify). CLAUDE.md maps reviewers to areas with effort levels; reviewer models pinned.
  Trial run of a one-line UI request followed it, except it committed without `just check` and
  edited via shell scripts → commit gate (tree fingerprint) + "edit with Edit/Write" rule.
- 2026-11-14 — output device following (human: AirPods connected, sound stayed on the Mac):
  listeners on default output + device list, rebuild tap + aggregate on UID/rate change,
  retry on failure, popover shows "Output: <device>" or why it's paused. Hardware test passes
  (AirPods ⇄ speakers, < 0.8 s).

- 2026-11-14 — demo recording: a loud close-mic smack (55 dB over the floor) was dropped as
  "too long" — its wet hiss + thump tail took 120 ms to settle. A settling-tail path passed its
  own regression test but the DSP review showed it ducks word-final stops released into a pause
  (same physics) — reverted, documented. Surprises: an envelope's release looks like a decaying
  tail; the thump's 60–120 Hz ripples a 2 ms envelope by > 6 dB; the stop test can't see
  released provisional ducks.

- 2026-11-14 — human review of snapshots: (1) de-smack missed a long multi-tick smack (latest
  clip) → re-open on ticks + provisional duck at the lookahead deadline; labelled clip now
  −17 dB; real labelled regions reduced ≥ 10 dB 5/23 → 17/23; stops/music/speech unchanged;
  dsp+rt review applied (released ducks not reported, end ≥ out_pos, 96 kHz, release test,
  block-size + alloc coverage). (2) Review window: delete clip (⌘⌫ / trash button / right-click
  → Trash) + `deleted.txt` so import skips deleted clips (fixed a shared temp-dir race in
  export/import on the way). (3) App menu also on right-click of the menu bar icon; Quit moved
  into it. Known limit: the first ~8 ms of a loud long smack (needs ≈ 50 ms lookahead).

- 2026-11-13 (overnight, human asleep) — waveform in popover; de-hum reworked (tracking,
  harmonic gating) and merged; de-ess, de-plosive, limiter built by parallel worktree agents,
  merged, reviewed (rt + dsp), robustness fixes applied; `just release` DMG + install guide;
  snapshot export/import/merge + device ids; `hearo dataset propose` (weak-label proposals on
  the human's 21 clips: 13 with a proposal in the press window); launch at login; settings
  persistence; UniFFI MPL-2.0 exception (ADR 0013, **human to confirm**); latency budget
  (ADR 0012). Later: de-ess onset gate (music false triggers 240/min → 0–1), de-click
  (LPC + AR, recall 1.0, below spec on repair depth), review/annotation window, settings
  persistence, de-click CPU fix + sample-rate-scaled IO buffer (64 @ 48 kHz), DMG
  `dist/hEARo-0.1.0-6c09b96.dmg`. 194 workspace tests + 41 Swift tests, `just ci` green.
  Morning checklist in OPEN_QUESTIONS.md.

- 2026-11-13 — M2 first slice (live engine via CLI). Surprise: a tap-backed aggregate's IOProc
  only runs while a tapped app plays. First misread as missing permission (the human hit it);
  TCC preflight said *authorized*, a sound made it run → idle state + real permission check.
  Live: 16–21 µs per 512-frame cycle (~0.2% of budget), 0 overruns.
- 2026-11-13 — M1 closed: review punch list fixed (NaN isolation, FTZ guard, block-size
  invariant gain ramp, 20+ new tests). Hooks now run via `mise x` (failed in non-mise shells).
  De-hum v1: band-energy hum measurement was speech-limited → superposition measurement;
  narrowing detectors 2 → 0.3 Hz gave hum/no-hum score separation (21–26 vs ≤ 11 dB).
- 2026-11-13 — M0/M1 bootstrap. Surprises: Xcode absent (CLT only) → SwiftPM-first app build
  (ADR 0004); `#Preview` and Swift Testing need Xcode; owner asked for no hosted CI (ADR 0009);
  chose a counting allocator over assert_no_alloc (ADR 0007). Swift test caught a locale bug
  (status line decimal separator) → AppModel takes an injectable Locale.
