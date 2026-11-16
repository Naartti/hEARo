# 0020. Phase A detector: features, model, data and evaluation

Date: 2026-11-16 · Status: accepted

## Context

M7 starts with 243 reviewed snapshots over four days. The split lock (ADR 0017, written by
the first real `just ml-import-snapshots` on 2026-11-16) puts two days in train (33 clips,
34 smacks, no hard negatives), the auto-save day in validation (193 clips, 55 smacks, 238
rejected catches) and one day in test (17 clips, 17 smacks). Train is far too small to learn
from alone. HANDOVER §7.6/§9 asks for a small causal CNN/GRU over 10 ms frames, focal loss,
codec augmentation, streaming parity ≤ 1e-4, and a report per run. De-smack already spends
40 ms of lookahead (ADR 0011). Verified: torch 2.15.0 (MPS on this Mac), numpy 2.5.5,
scipy 1.18.1, soundfile 0.14.0, pyyaml 6.0.3; ffmpeg with libopus and aac.

## Decision

We run features at 32 kHz: 20 ms Hann window, 10 ms hop, 48 mel bands (50 Hz to 16 kHz),
dB, and every frame strictly causal so Rust can compute the same frames (M8). The detector is
two causal 1-D convolutions, a GRU and a linear head; 32 channels and 32 GRU units with
dropout 0.3 (≈ 14 k parameters; the first 64/64 run without dropout overfit after 500 steps).
It gets a **30 ms lookahead**: output *t* labels frame *t − 3*, which fits inside de-smack's
40 ms and so costs no extra latency once it gates the repair. A **bidirectional teacher** of
the same width trains on the same data as the offline upper bound, and later ranks
auto-saved clips for review (it never deletes them). To get enough training data, each train
clip gets 12 mixes: train-day smacks plus synthetic smacks/clicks (`hearo gen-signals`, train
seeds; 60 % of draws are real; each keeps its class) pasted in at −20 to +10 dB relative to
the speech, then EQ tilt, gain, and Opus 16/24/32 or AAC round-trips on 60 % of mixes,
realigned by cross-correlation. The synthetic clean speech and music are never trained on:
they're test-only, 5 min per seed, for false positives per minute. Rejected spans of
de-smack's catches weigh 4× as negatives (none fall on train days yet, so today this does
nothing). The checkpoint and threshold are picked on validation event F1.
`just ml-eval` scores validation with the rules of `hearo dataset eval` (ported and tested;
close detections merge into one event on both sides), puts the DSP de-smack next to the
model on the same clips with a note on how each split's clips were chosen, and adds Wilson
intervals and Poisson upper bounds. Test is scored only on request (`TEST=1`), and the number
of looks is counted per run.

## Alternatives considered

- **Public speech corpora + Freesound events now** (HANDOVER §9.1): more varied data, but
  downloads of gigabytes and licence bookkeeping before we know whether the approach works
  on our own audio. It's the next data step if real train days stay scarce.
- **Re-splitting so the auto-save day trains**: more data today, but it breaks the lock that
  keeps tuning honest, and validation would lose its hard negatives.
- **48 kHz / linear STFT bins**: more input, a bigger model, and the extra band is mostly
  removed by meeting codecs anyway.
- **Zero lookahead**: the streaming model would have to call a smack before the quiet after
  it, which is the very cue that tells smacks from stops (ADR 0011).

## Consequences (incl. how we'd know it was wrong)

- `just ml-retrain` re-imports, rebuilds the data cache (keyed by manifest hash, data
  config, the data code and tool versions) and retrains both configs, so new review sessions
  feed straight in.
- With 17 test smacks the 95 % interval on recall is about ±20 points: until test passes
  50 events (warned in every report), runs are ranked on validation only.
- **The DSP comparison is biased by how clips were chosen.** Test clips are smacks de-smack
  *missed* (its recall there is low by design); validation is mostly its own auto-saved
  catches (its recall there is high). Neither is a fair head-to-head. That needs clips saved at
  random times or continuous listening, and HANDOVER §9.3's "annoyance presses per hour".
- Synthetic events come from the same generator as the synthetic training events, so the
  synthetic F1 is in-distribution; the synthetic speech and music FP rates aren't.
- Validation picks the checkpoint and threshold, so its numbers are optimistic.
- Wrong if: the teacher can't beat the DSP de-smack on validation F1 once train has more
  than ~150 real events (then the features or the labels are the problem, not the amount of
  data), or the streaming detector trails the teacher by more than ~15 F1 points (then the
  30 ms lookahead is too short).
