# 0017. Mouth-noise dataset from hotkey snapshots, not a recording session

Date: 2026-11-14 · Status: accepted

## Context

HANDOVER §9.1 plans three mouth-noise sources: a guided recording session of the user making
smacks, Freesound clips, and hotkey snapshots. The human (2026-11-14): different people smack
differently, so their own deliberate smacks would teach one person's sound; snapshots captured
while working and in meetings over a few days give many speakers, mics, rooms and codecs — the
audio hEARo actually processes. Facts: snapshots are 8 s of *unprocessed* system audio with a
sidecar holding exact `regions` and a `reviewed` flag (Review tab, M6 early); they stay local
because they contain other people's voices (§13.5). 34 labelled regions in 33 snapshots so far.

## Decision

We build the dataset from hotkey snapshots. A snapshot whose `reviewed` is true is
**exhaustively labelled**: every mouth noise in its 8 s has a region, so all unmarked audio is a
negative and no separate negative labelling is needed. Clean, isolated events from training
snapshots (quiet before and after) are the source pool for synthetic mixing over other speech
and noise; Freesound CC0/CC-BY clips widen it. Splits are by recording day/session, and the test
set is real, unmixed snapshots, including overlapped smacks. The full rules are in
docs/ml/README.md. The guided recording session is dropped.

## Alternatives considered

- **Guided recording session** (the brief): one speaker, one mic, deliberate (unnatural) smacks;
  fast to collect but narrow. Kept only as an option if the pool stays too small.
- **Snapshots with weak labels only** (the press time, "0.5–2 s before"): too loose for a
  frame-level detector; exact regions cost ~20–30 s per clip.
- **Public mouth-noise corpora**: none found with a usable licence and lip-smack labels; Freesound
  is the supplement.

## Consequences (incl. how we'd know it was wrong)

- Labelling happens in daily use; the dataset grows by roughly 5–20 events per meeting hour.
- "Reviewed" now carries a promise (every event marked); a half-reviewed clip would teach
  smacks as negatives. Clips not exhaustively labelled are test-only or skipped.
- The dataset (and likely a model trained on it) can't be published: colleagues' voices.
- The mixing pool is biased to smacks in pauses — the case the DSP de-smack already handles —
  so the overlap case is only measured honestly on the real test set.
- Wrong if: the held-out real test set shows recall on overlapped smacks well below recall on
  synthetic mixes (≥ 0.2 gap), or the pool has < 150 clean events after two weeks of capture —
  then add the recording session or more Freesound clips.
