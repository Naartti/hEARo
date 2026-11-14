---
name: ml-experiment
description: How to configure, run, log, evaluate and report a hEARo training run, and how to promote a model. Use for any ML training or evaluation work under ml/.
---

# Running an ML experiment

1. Config in `ml/configs/<name>.yaml` (never hard-code hyperparameters).
2. `just ml-train CONFIG=ml/configs/<name>.yaml` — logs config, git SHA, dataset manifest hash
   and metrics to `ml/runs/<timestamp>-<name>/` (git-ignored; local only, no cloud trackers).
3. `just ml-eval RUN=<run>` writes `docs/ml/reports/<run>.md` with plots and a comparison to the
   current best model. Commit the report.
4. Check: detection F1 ≥ 0.8 on the synthetic test set; FP on music ≤ 2/min; streaming parity
   ≤ 1e-4. Have `ml-reviewer` review before promoting.
5. Promote: `just ml-export RUN=<run>` → `models/<name>/model.onnx` (< 5 MB) + MODEL_CARD.md,
   then run the Rust parity test. Commit `feat(ml): promote <name>` with the card and report.
