# M7: Phase A detector training and evaluation pipeline

Belongs to PROGRESS.md *M7 — ML pipeline* (HANDOVER §14 M7). Closes the M6 manifest
follow-ups "M7 eval: report Wilson intervals; refuse test metrics while validation is empty".

Human request (2026-11-16): "start working on M7 and the whole training and evaluation and
such. And then we can re-run it in an easy manner when new data arrives later. Also I'm
curious how it'll go with the current dataset." The elastic-latency idea is parked ("forget
about the elastic buffer for now to see how the ML approach will work").

Acceptance:

- One command re-runs everything on new data: `just ml-retrain` (import → data → train every
  config → report).
- Training is config-driven (`ml/configs/*.yaml`), logs config, git SHA, manifest hash and
  metrics to `ml/runs/`; data is cached by manifest hash + data config.
- The report compares the model with the DSP de-smack on the same clips by the same rules as
  `hearo dataset eval`, with Wilson intervals; test metrics are withheld while validation is
  empty and flagged below 50 events; no clip names in committed reports.
- Streaming detector is causal and matches its frame-by-frame `step` within 1e-4 (test).
- A first real run on today's dataset, with its report committed.

Done: `hearo dataset eval --json`; `hearo_ml.{features,scoring,dataset,model,train,evaluate}`
with tests (features causality, Rust-mirrored scoring, labels, mixing, codec alignment,
streaming parity, end-to-end build → train → report); `just ml-data/ml-train/ml-eval/
ml-retrain`; configs `detector` (30 ms lookahead) and `teacher` (bidirectional); ADR 0020;
docs/ml/README.md *Training and evaluation*. First split lock written 2026-11-16 (train
11-13 + 11-16, val 11-15, test 11-14).

Results (2026-11-16, reports in docs/ml/reports/, test scored once per model):

| | val F1 | val recall / precision | val FP/min | test F1 (16 events) | synthetic F1 | synthetic music FP/min |
|---|---|---|---|---|---|---|
| detector (streaming, 14 k params) | 43 % | 45 % / 41 % | 1.5 | 75 % | 65 % | 220 |
| teacher (offline) | 43 % | 44 % / 42 % | 1.4 | 76 % | 89 % | 232 |
| DSP de-smack, same clips | 28 % | 78 % / 17 % | 8.6 | 29 % | | |

Reading it: the model trades recall for far fewer false hits (4 vs 176 on your rejected
spans). The DSP comparison is biased both ways by how clips were chosen (test = de-smack's
misses, val = its catches). The streaming detector matches the teacher on real data, so
30 ms of lookahead isn't the limit; the amount of data is. M7's targets aren't met: synthetic
F1 65 % (< 80 %), and on held-out synthetic music it fires ~220 times per minute (target ≤ 2),
because it has never heard music. Not ready to run live.

Tuning: one round, on validation only (64/64 units overfit after 500 steps; 32/32 + dropout
0.3 + 1500 steps). Test was looked at during the first two rounds' reports, before it was made
opt-in; noted here for honesty.

Review notes: code-review (medium) found 10 issues, all fixed. ml-reviewer (high) fixed: split
notes on the DSP bias, synthetic noise taken out of train and 5 min test seeds, test on
request with a view count, Poisson upper bounds, val sweep instead of the redundant "oracle"
line, last step kept without val events, code and tool versions in the cache key, the dirty
check scoped, pasted events keep their class, the comment on feature scaling, a tighter codec
lag test, `ml-retrain` evaluating its own run. Not done: L3/L4 (Python and Rust handle a
missing region class and out-of-clip spans differently; no current clip has either), L6 and
L12 (below), L1 (Wilson intervals ignore clustering; noted in every report).

## Follow-ups

- [ ] **Music negatives:** real CC0/CC-BY music (e.g. FMA, Freesound) as training negatives and
      a held-out real-music FP set. ~220 FP/min on synthetic music today; blocks live use.
- [ ] ONNX export + model card + Python/Rust parity fixtures (`just ml-export`; rest of M7).
      Parity fixtures start after resampling: `resample_poly` is zero-phase; the Rust runtime
      needs a causal resampler, whose delay counts against the lookahead budget.
- [ ] More real training data: only 34 train-day smacks, no train-day hard negatives (every
      auto clip so far is on the validation day). Public speech (LibriSpeech/VCTK) backgrounds
      and Freesound CC0 events (HANDOVER §9.1) if review days stay scarce.
- [ ] A fair DSP comparison: clips saved at random times (not around misses or catches), and
      HANDOVER §9.3's "annoyance presses per listening hour".
- [ ] Use the teacher to rank auto-saved clips in Review (never to delete them: rejected
      catches are the hard negatives).
- [ ] Per-speaker/show group tag so splits can follow people, not just days (ADR 0017 limit).
- [ ] Align Python and Rust on a missing region class (Rust: neutral, Python: lip smack) and
      out-of-clip spans; pick the validation threshold on CPU like evaluation (MPS rounding).
