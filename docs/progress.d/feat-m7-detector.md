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

Results: see the reports in docs/ml/reports/ (summary below).

## Follow-ups

- [ ] ONNX export + model card + Python/Rust parity fixtures (`just ml-export`; rest of M7).
- [ ] More real training data: only 34 train-day smacks. Public speech (LibriSpeech/VCTK)
      backgrounds and Freesound CC0 events (HANDOVER §9.1) if review days stay scarce.
- [ ] Use the teacher to rank auto-saved clips in Review (never to delete them: rejected
      catches are the hard negatives).
- [ ] Per-speaker/show group tag so splits can follow people, not just days (ADR 0017 limit).
