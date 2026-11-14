# 0015. Module settings owned by Rust; per-module gain reduction in waveform columns

Date: 2026-11-14 · Status: accepted

## Context

The UI is getting per-module "how much" controls, advanced parameters, a review-window render
with module toggles, and a waveform coloured by what each module removed. That needs (a) a
persisted settings format that survives app updates in both directions and engine rebuilds on
device changes, and (b) per-column, per-module gain reduction from the audio thread without
allocation or locks, at 60 columns/s for ~68 s of history.

## Decision

**Settings.** Rust owns the meaning of settings. `EngineSettings { modules: [{ key, enabled,
params: [{ key, value }] }] }` is a plain UniFFI record; Swift persists the JSON from
`settings_to_json` and reads it back with `settings_from_json` (serde: unknown fields/modules/
params ignored, missing ones defaulted, values clamped and snapped with `ParamDesc::snap`). The
FFI engine keeps a `ModuleState` (enabled + snapped values per slot) and pushes all of it into the
lock-free `Controls` atomics on every (re)start, so settings survive device-change rebuilds.
Global bypass is not a setting. The limiter is always on.

**Reduction columns.** Each live column stores the original peak and processed peak as u16 and,
for the five cleanup modules (`ModuleId::CLEANUP`, chain order), the column's largest gain
reduction as a u8 in 0.25 dB steps (0–63.75 dB), in two `AtomicU64`. Each module's meter is
delayed by the lookahead of the slots after it, so all rows line up with the output. Offline
renders compute the same rows exactly on the clip timeline (`hearo_dsp::offline`).

## Alternatives considered

- **Swift `Codable` as the source of truth** — two places to keep in sync with module params;
  Rust already knows ranges, steps and choices.
- **Settings stored by Rust on disk** — would add file I/O and a path policy to the engine; the
  app already owns preferences.
- **f32 per module per column / a seqlock'd struct** — 4× the memory and a retry loop on the
  reader; 0.25 dB is far below what a waveform colour can show.
- **Only a "removed" bit** (previous) — can't colour per module or show depth.

## Consequences (incl. how we'd know it was wrong)

- Adding a module or param needs no migration: old files load with defaults.
- More than 8 cleanup modules would need a third word (compile-time assert).
- Reductions above 63.75 dB saturate (de-hum can report up to its tonal score); fine for colour,
  wrong if someone uses the column data as a meter — the per-module `gain_reduction_db` in
  `EngineStatus` stays f32.
- Wrong if users report settings lost after an update, or columns visibly misaligned with the
  smack they belong to (alignment is to ±½ column = ±8 ms).
