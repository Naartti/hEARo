### Added

- `hearo dataset eval --json` prints each reviewed clip's detections as one JSON line, so the
  ML report can compare the de-smack module and a trained model on the same clips.
- A first trainable mouth-noise detector: `just ml-retrain` turns your reviewed snapshots into
  training data, trains the streaming detector and an offline "teacher" model, and writes a
  report comparing both with the de-smack module (`docs/ml/reports/`). Run it again whenever
  you've reviewed more clips.
