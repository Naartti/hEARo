# 0018. Auto-save de-smack catches (opt-in)

Date: 2026-11-14 · Status: accepted

## Context

ADR 0017 builds the dataset from hotkey snapshots. The human (2026-11-14): de-smack already
finds smacks live, so save a clip around each catch, pre-marked and unverified, and verify it
in the Review tab with a key press — far faster than waiting for misses. Requested: a Settings
toggle (default off), an orange ear flash on save, a daily cap (default 250) and auto-delete
(default 31 days), both adjustable; nearby catches share a clip; rejected detections kept as
negatives. Privacy: fine for them, everything stays local. Until now hEARo stored audio only on
a key press, and the permission text said so. The chain's events were dropped on the audio
thread.

## Decision

The IOProc copies chain events into a lock-free SPSC ring (`hearo_engine::events`). The
`hearo-supervisor` thread drains it every 0.5 s (also while the feature is off, so it never
fills) and runs a pure policy (`hearo_engine::autocapture`): de-smack catches within 4.5 s share a
burst; once 3 s after its last catch are captured, the 8 s window is cut from the capture ring by
absolute frame (chain input clock = capture clock) and saved as label `auto` with the catches as
`proposals`. A rolling 24 h cap (seeded from the files on disk) drops clips beyond it; untouched
auto clips (not reviewed, no regions, nothing rejected) older than the retention are deleted,
hourly and when settings change. The engine is rebuilt on device changes, so pending bursts are
forgotten when its generation changes. Settings live in the app's preferences; Rust clamps them.
In Review, accepted proposals become regions and rejected ones go to a `rejected` list — the
hard negatives.

## Alternatives considered

- **Only proposals on hotkey snapshots** (`hearo dataset propose`, already there): no extra
  audio stored, but grows only as fast as the human presses keys.
- **Continuous recording** with later mining: far more data, far more privacy exposure and disk.
- **Saving on the audio thread or in the IOProc's wake**: file I/O on the RT thread is
  forbidden; a 0.5 s control-thread tick is plenty for a 3 s post-roll.

## Consequences (incl. how we'd know it was wrong)

- Data grows with listening time (up to 250 clips ≈ 750 MB per day at 48 kHz float stereo).
- Auto clips only contain what de-smack already catches; they fill the mixing pool and, via
  rejections, the hard-negative set — not the overlap cases (ADR 0017's caveats hold).
- The permission text and Settings now name a second, opt-in exception to "nothing is stored".
- Wrong if: the acceptance rate of auto proposals is so low that reviewing them costs more than
  it yields (< 30 % accepted over 200 clips), or the cap is routinely hit before noon (then the
  default or the grouping is off).
