"""Train the Phase A detector from a config (`just ml-train CONFIG=ml/configs/<name>.yaml`).

The config (YAML) has three sections — `data` (`dataset.DataConfig`), `model`
(`model.DetectorConfig`) and `train` (`TrainConfig`); nothing is hard-coded here. A run writes
to `ml/runs/<timestamp>-<name>/` (git-ignored):

- `config.yaml` as given, `meta.json` (git SHA, manifest SHA-256, data cache key, versions,
  parameter count), `history.jsonl` (loss and validation score per evaluation),
- `model.pt`: the checkpoint with the best **validation** event F1, and `best.json` (its step
  and the decision threshold picked on validation). Test data is never looked at here.

Loss: focal loss (HANDOVER §9.2) on frames, weighted per frame (`dataset.frame_targets`:
unscored edges and neutral regions weigh 0, hard negatives more), with the targets shifted by
the model's lookahead.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

import numpy as np
import torch
import yaml

from hearo_ml import dataset as ds
from hearo_ml import features as feat
from hearo_ml.model import Detector, DetectorConfig
from hearo_ml.scoring import Score, detections, score

THRESHOLDS = tuple(round(t, 2) for t in np.arange(0.1, 0.96, 0.05))


@dataclass(frozen=True)
class TrainConfig:
    steps: int = 3000
    batch: int = 32
    crop_frames: int = 400
    lr: float = 2e-3
    weight_decay: float = 1e-4
    focal_gamma: float = 2.0
    focal_alpha: float = 0.75
    #: Share of crops centred on a target frame (events are rare: ~1 % of frames).
    positive_crops: float = 0.5
    eval_every: int = 250
    seed: int = 0
    device: str = "auto"


def focal_loss(logits: torch.Tensor, y: torch.Tensor, w: torch.Tensor, gamma: float, alpha: float) -> torch.Tensor:
    p = torch.sigmoid(logits)
    ce = torch.nn.functional.binary_cross_entropy_with_logits(logits, y, reduction="none")
    pt = torch.where(y > 0.5, p, 1 - p)
    a = torch.where(y > 0.5, torch.full_like(y, alpha), torch.full_like(y, 1 - alpha))
    loss = a * (1 - pt) ** gamma * ce * w
    return loss.sum() / w.sum().clamp_min(1.0)


def pick_device(name: str) -> torch.device:
    if name != "auto":
        return torch.device(name)
    return torch.device("mps" if torch.backends.mps.is_available() else "cpu")


class Batches:
    """Random fixed-length crops; a share of them centred on a target frame."""

    def __init__(self, items: list[ds.Item], cfg: TrainConfig, rng: np.random.Generator) -> None:
        self.items = [i for i in items if len(i.feats) > 0]
        self.cfg, self.rng = cfg, rng
        lens = np.array([len(i.feats) for i in self.items], dtype=np.float64)
        self.p = lens / lens.sum()
        self.pos = [(k, t) for k, i in enumerate(self.items) for t in np.flatnonzero(i.y > 0.5)]

    def __call__(self) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
        c, n = self.cfg.crop_frames, self.cfg.batch
        nm = self.items[0].feats.shape[1]
        x = np.full((n, c, nm), feat.LOG_FLOOR_DB, dtype=np.float32)
        y = np.zeros((n, c), dtype=np.float32)
        w = np.zeros((n, c), dtype=np.float32)
        for b in range(n):
            if self.pos and self.rng.random() < self.cfg.positive_crops:
                k, t = self.pos[int(self.rng.integers(len(self.pos)))]
                start = t - int(self.rng.integers(c // 4, 3 * c // 4))
            else:
                k = int(self.rng.choice(len(self.items), p=self.p))
                start = int(self.rng.integers(-c // 4, max(1, len(self.items[k].feats) - c // 2)))
            it = self.items[k]
            a, z = max(0, start), min(len(it.feats), start + c)
            if z > a:
                x[b, a - start : z - start] = it.feats[a:z]
                y[b, a - start : z - start] = it.y[a:z]
                w[b, a - start : z - start] = it.w[a:z]
        return x, y, w


def score_probs(probs: dict[str, np.ndarray], items: list[ds.Item], threshold: float) -> Score:
    """Event score of per-frame probabilities (`probs[item.name]`) over `items`."""
    total = Score()
    for it in items:
        total.add(score(detections(probs[it.name], threshold, feat.HOP_S), it.labels, it.seconds))
    return total


def best_threshold(probs: dict[str, np.ndarray], items: list[ds.Item]) -> tuple[float, Score]:
    """The threshold with the highest event F1 (ties: the higher threshold, fewer false hits)."""
    best = (THRESHOLDS[0], Score())
    best_f1 = -1.0
    for t in THRESHOLDS:
        s = score_probs(probs, items, t)
        f1 = s.f1() or 0.0
        if f1 >= best_f1:
            best, best_f1 = (t, s), f1
    return best


def git_sha() -> str:
    """HEAD, with `+dirty` when tracked or new files differ from it (the run used that code)."""
    try:
        git = ["git", "-C", str(ds.REPO)]
        sha = subprocess.run([*git, "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
        # Only what shapes a run: the ML code and configs, the Rust that generates signals
        # and scores the baseline. (Untracked reports or notes elsewhere don't count.)
        paths = ["ml/hearo_ml", "ml/configs", "ml/pyproject.toml", "ml/uv.lock", "crates", "Cargo.lock"]
        status = subprocess.run(
            [*git, "status", "--porcelain", "--", *paths], capture_output=True, text=True, check=True
        ).stdout
        return sha + ("+dirty" if status.strip() else "")
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def load_config(path: Path) -> tuple[str, ds.DataConfig, DetectorConfig, TrainConfig]:
    raw = yaml.safe_load(path.read_text())
    return (
        raw.get("name", path.stem),
        ds.config_from_dict(raw.get("data", {})),
        DetectorConfig(**raw.get("model", {})),
        TrainConfig(**raw.get("train", {})),
    )


def train(
    config: Path,
    data_folder: Path,
    runs: Path,
    log=print,
    now: dt.datetime | None = None,
) -> Path:
    """Train from `config` on the built sets in `data_folder`; returns the run folder."""
    name, dcfg, mcfg, tcfg = load_config(config)
    torch.manual_seed(tcfg.seed)
    rng = np.random.default_rng(tcfg.seed)
    train_items, val_items = ds.load(data_folder, "train"), ds.load(data_folder, "val")
    val_events = sum(len(it.labels.targets) for it in val_items)
    if not train_items:
        raise SystemExit("✖ no training clips")
    stamp = (now or dt.datetime.now()).strftime("%Y%m%d-%H%M%S")
    run = runs / f"{stamp}-{name}"
    run.mkdir(parents=True, exist_ok=False)
    (run / "config.yaml").write_text(config.read_text())
    done = json.loads((data_folder / "done.json").read_text())

    dev = pick_device(tcfg.device)
    model = Detector(mcfg)
    allf = np.concatenate([i.feats for i in train_items])
    model.set_normalisation(allf.mean(axis=0), allf.std(axis=0))
    model.to(dev)
    meta = {
        "name": name,
        "git_sha": git_sha(),
        "manifest_sha256": done["manifest_sha256"],
        "data_key": done["key"],
        "data_sets": done["sets"],
        "torch": torch.__version__,
        "device": str(dev),
        "params": model.n_params(),
        "data": asdict(dcfg),
        "model": asdict(mcfg),
        "train": asdict(tcfg),
    }
    (run / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    log(f"run {run.name}: {model.n_params()} parameters on {dev}, {len(train_items)} train clips")

    opt = torch.optim.AdamW(model.parameters(), lr=tcfg.lr, weight_decay=tcfg.weight_decay)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=tcfg.lr, total_steps=max(1, tcfg.steps))
    batches = Batches(train_items, tcfg, rng)
    k = model.shift
    best_f1, history = -1.0, []
    for step in range(1, tcfg.steps + 1):
        model.train()
        x, y, w = (torch.as_tensor(a, device=dev) for a in batches())
        logits = model(x)
        if k:
            logits, y, w = logits[:, k:], y[:, :-k], w[:, :-k]
        loss = focal_loss(logits, y, w, tcfg.focal_gamma, tcfg.focal_alpha)
        opt.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        opt.step()
        sched.step()
        if step % tcfg.eval_every == 0 or step == tcfg.steps:
            model.eval()
            loss = loss.detach()
            probs = {it.name: model.predict(it.feats) for it in val_items}
            thr, s = best_threshold(probs, val_items) if val_items else (0.5, Score())
            f1 = s.f1() or 0.0
            entry = {
                "step": step,
                "loss": round(float(loss), 5),
                "val_f1": round(f1, 4),
                "val_recall": s.recall(),
                "val_precision": s.precision(),
                "threshold": thr,
            }
            history.append(entry)
            with (run / "history.jsonl").open("a") as f:
                f.write(json.dumps(entry) + "\n")
            log(f"  step {step:5d}  loss {float(loss):.4f}  val F1 {f1:.3f} @ {thr}")
            # Without validation events every F1 is 0: keep the last step, not the first.
            if f1 > best_f1 or not val_events:
                entry["selected_by"] = "val_f1" if val_events else "last_step (validation has no events)"
                best_f1 = f1
                torch.save(model.to_checkpoint(), run / "model.pt")
                (run / "best.json").write_text(json.dumps(entry, indent=2) + "\n")
    return run


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="hearo_ml.train", description=__doc__.splitlines()[0])
    ap.add_argument("--config", type=Path, default=ds.ML_DIR / "configs/detector.yaml")
    ap.add_argument("--manifest", type=Path, default=ds.ML_DIR / "data/processed/snapshots/manifest.jsonl")
    ap.add_argument("--data", type=Path, default=ds.ML_DIR / "data/processed/features")
    ap.add_argument("--runs", type=Path, default=ds.ML_DIR / "runs")
    args = ap.parse_args(ds.cli_args(sys.argv[1:] if argv is None else argv))
    if not args.manifest.exists():
        print(f"✖ {args.manifest} missing: run `just ml-import-snapshots` first", file=sys.stderr)
        return 1
    _, dcfg, _, _ = load_config(args.config)
    import shutil

    folder = ds.build(args.manifest, args.data, dcfg, ds.default_hearo(), shutil.which("ffmpeg"))
    run = train(args.config, folder, args.runs)
    print(f"✔ run {run}")  # `just ml-retrain` reads this line to evaluate exactly this run
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
