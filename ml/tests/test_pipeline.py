"""End to end on a tiny fake manifest: build the sets → train a few steps → evaluate → report."""

from __future__ import annotations

import datetime as dt
import json
from pathlib import Path

import numpy as np
import soundfile as sf

from hearo_ml import dataset as ds
from hearo_ml import evaluate as ev
from hearo_ml import features as feat
from hearo_ml import train as tr
from hearo_ml.scoring import Span

CONFIG = """
name: tiny
data:
  mixes_per_clip: 1
  events_per_mix: [1, 2]
  synthetic_train_seeds: []
  synthetic_test_seeds: []
model: {channels: 4, hidden: 4, lookahead: 1}
train: {steps: 4, batch: 2, crop_frames: 50, eval_every: 2, device: cpu}
"""


def clip_audio(seed: int, smacks: list[float]) -> np.ndarray:
    rng = np.random.default_rng(seed)
    x = 0.003 * rng.standard_normal(3 * 48_000).astype(np.float32)
    for t in smacks:
        a = int(t * 48_000)
        x[a : a + 480] += 0.3 * np.hanning(480).astype(np.float32) * np.sign(rng.standard_normal(480))
    return x


def fake_manifest(root: Path, val_events: bool = True) -> Path:
    snaps = root / "snaps"
    snaps.mkdir()
    lines = []
    for stem, split, smacks, auto in [
        ("snapshot-1-missed", "train", [1.0, 2.0], False),
        ("snapshot-2-auto", "val", [1.5] if val_events else [], True),
        ("snapshot-3-missed", "test", [1.2], False),
    ]:
        wav = snaps / f"{stem}.wav"
        sf.write(wav, np.stack([clip_audio(len(lines), smacks)] * 2, axis=1), 48_000, subtype="FLOAT")
        regions = [{"start_s": t, "end_s": t + 0.01, "cls": "lip_smack"} for t in smacks]
        rejected = [{"start_s": 2.5, "end_s": 2.52, "cls": "rejected"}] if auto else []
        lines.append(
            {"stem": stem, "wav": str(wav), "split": split, "auto": auto, "regions": regions, "rejected": rejected}
        )
    out = root / "manifest"
    out.mkdir()
    (out / "manifest.jsonl").write_text("".join(json.dumps(line) + "\n" for line in lines))
    (out / "meta.json").write_text(json.dumps({"manifest_sha256": "f" * 64}))
    return out / "manifest.jsonl"


def run_all(tmp_path: Path, val_events: bool = True) -> tuple[Path, Path]:
    manifest = fake_manifest(tmp_path, val_events)
    cfg_path = tmp_path / "tiny.yaml"
    cfg_path.write_text(CONFIG)
    _, dcfg, _, _ = tr.load_config(cfg_path)
    logs: list[str] = []
    folder = ds.build(manifest, tmp_path / "features", dcfg, hearo=None, ffmpeg=None, log=logs.append)
    assert ds.build(manifest, tmp_path / "features", dcfg, None, None, log=logs.append) == folder
    assert any("is current" in line for line in logs), "a second build reuses the cache"
    run = tr.train(cfg_path, folder, tmp_path / "runs", log=logs.append, now=dt.datetime(2026, 11, 16, 12))
    dsp = {"snapshot-1-missed": [], "snapshot-2-auto": [Span(1.5, 1.51)], "snapshot-3-missed": [Span(0.9, 0.91)]}
    report = ev.run_eval(run, tmp_path / "features", tmp_path / "reports", lambda: dsp, log=logs.append)
    return run, report


def test_build_train_evaluate_report(tmp_path):
    run, report = run_all(tmp_path)
    assert run.name == "20261116-120000-tiny"
    for f in ("config.yaml", "meta.json", "history.jsonl", "model.pt", "best.json", "metrics.json", "dsp.json"):
        assert (run / f).exists(), f
    meta = json.loads((run / "meta.json").read_text())
    assert meta["manifest_sha256"] == "f" * 64 and meta["params"] > 0
    metrics = json.loads((run / "metrics.json").read_text())
    dsp_val = metrics["splits"]["val"]["dsp"]
    assert (dsp_val["hit_regions"], dsp_val["regions"], dsp_val["false_positives"]) == (1, 1, 0)
    assert metrics["splits"]["test"]["dsp"]["false_positives"] == 1
    text = report.read_text()
    assert "## val" in text and "## test" in text and "DSP de-smack" in text
    assert "snapshot-" not in text, "reports never name clips"
    assert "test has 1 events" in text


def test_train_items_include_mixes_with_more_events(tmp_path):
    manifest = fake_manifest(tmp_path)
    folder = ds.build(manifest, tmp_path / "f", ds.DataConfig(mixes_per_clip=2, synthetic_train_seeds=()), None, None)
    train = ds.load(folder, "train")
    assert [i.kind for i in train] == ["real", "mix", "mix"]
    assert all(i.y.sum() > train[0].y.sum() for i in train[1:])
    assert all(len(i.feats) == feat.n_frames(3 * feat.SR) for i in train)


def test_test_metrics_withheld_without_validation_events(tmp_path):
    run, report = run_all(tmp_path, val_events=False)
    metrics = json.loads((run / "metrics.json").read_text())
    assert "test" not in metrics["splits"]
    assert "test metrics withheld" in report.read_text()
