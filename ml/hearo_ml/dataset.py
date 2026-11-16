"""Training and evaluation sets for the detector (`just ml-data`; HANDOVER §9.1, ADR 0017).

From the snapshot manifest (`just ml-import-snapshots`) and the synthetic reference signals
(`hearo gen-signals`), this builds feature sets in a git-ignored cache keyed by the manifest
hash and the data config, so a rebuild only happens when either changes:

- **train**: every train-split clip as recorded, plus `mixes_per_clip` *mixes* of it — mouth
  noises from the *event bank* (every train-day region, which includes the pool, plus the
  synthetic train events; never val/test days) pasted in
  at random times and event-to-speech ratios, then gain, EQ tilt and a codec round-trip
  (Opus 16 to 32 kbit/s, AAC) — plus synthetic speech with smacks/clicks from train seeds.
- **val / test**: the real clips of those days, unmixed (they're codec-processed already).
- **synthetic_test**: `gen-signals` from separate seeds — speech with smacks and clicks (exact
  labels), clean speech and music (false positives per minute; HANDOVER M7 targets).

Frame labels (`frame_targets`): 1 on target regions, 0 elsewhere; neutral regions (breath, …)
and one frame either side of a target are not scored (hand-drawn edges); human-rejected spans
(hard negatives) weigh `hard_negative_weight`.

I/O stays at the edges: `build` takes the manifest path, the output folder and the `hearo` /
`ffmpeg` commands, so tests run it on a temp dir without either.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pickle
import shutil
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass, field
from pathlib import Path

import numpy as np
from scipy.signal import lfilter

from hearo_ml import features as feat
from hearo_ml.scoring import TARGET_CLASSES, Labels, Span, is_own_rejection, labels_from_clip

#: Bump when a change here changes the built sets (it's part of the cache key).
FORMAT = 1
ML_DIR = Path(__file__).resolve().parents[1]
REPO = ML_DIR.parent


@dataclass(frozen=True)
class DataConfig:
    mixes_per_clip: int = 12
    events_per_mix: tuple[int, int] = (2, 6)
    #: Event-to-speech ratio range (dB, event RMS vs the clip's active RMS), HANDOVER §9.1.
    esr_db: tuple[float, float] = (-20.0, 10.0)
    gain_db: tuple[float, float] = (-12.0, 6.0)
    #: Share of mixes that go through each codec; the rest stay uncompressed.
    codecs: tuple[tuple[str, int, float], ...] = (
        ("opus", 16, 0.15),
        ("opus", 24, 0.15),
        ("opus", 32, 0.15),
        ("aac", 96, 0.15),
    )
    synthetic_train_seeds: tuple[int, ...] = (101, 102, 103, 104)
    synthetic_test_seeds: tuple[int, ...] = (1, 2, 3)
    synthetic_secs: float = 30.0
    hard_negative_weight: float = 4.0
    #: How often a mix draws a real (train-day) event rather than a synthetic one.
    real_event_share: float = 0.6
    seed: int = 0


@dataclass
class Item:
    """One clip's features and labels."""

    name: str
    kind: str  # real | mix | synthetic
    split: str
    feats: np.ndarray
    y: np.ndarray
    w: np.ndarray
    labels: Labels
    seconds: float
    auto: bool = False
    #: For synthetic_test: what the clip measures (events | speech | music).
    role: str = "events"
    extra: dict = field(default_factory=dict)


# ── labels ──────────────────────────────────────────────────────────────────────────────────


def _frames_of(n: int, start_s: float, end_s: float) -> range:
    a = max(0, int(np.floor(start_s / feat.HOP_S)))
    b = min(n, int(np.ceil(end_s / feat.HOP_S)))
    return range(a, max(a, b))


def frame_targets(
    n: int, regions: list[dict], rejected: list[dict], hard_negative_weight: float = 4.0, margin: int = 1
) -> tuple[np.ndarray, np.ndarray]:
    """Per-frame target and weight (see the module doc). Frame `t` = `[t, t + 1) * HOP_S`."""
    y = np.zeros(n, dtype=np.float32)
    w = np.ones(n, dtype=np.float32)
    for r in rejected:
        if is_own_rejection(r):
            w[list(_frames_of(n, r["start_s"], r["end_s"]))] = hard_negative_weight
    for r in regions:
        if r["cls"] in TARGET_CLASSES:
            continue
        w[list(_frames_of(n, r["start_s"], r["end_s"]))] = 0.0
    for r in regions:
        if r["cls"] not in TARGET_CLASSES:
            continue
        fr = _frames_of(n, r["start_s"], r["end_s"])
        if len(fr) == 0:
            fr = _frames_of(n, r["start_s"], r["start_s"] + feat.HOP_S)
        lo, hi = max(0, fr.start - margin), min(n, fr.stop + margin)
        w[lo:hi] = 0.0
        y[fr.start : fr.stop] = 1.0
        w[fr.start : fr.stop] = 1.0
    return y, w


# ── mixing ──────────────────────────────────────────────────────────────────────────────────


def _rms_db(x: np.ndarray) -> float:
    return 10.0 * np.log10(float(np.mean(x.astype(np.float64) ** 2)) + 1e-12)


def active_rms_db(x: np.ndarray) -> float:
    """RMS over the 10 ms hops louder than -50 dBFS (the speech, not the pauses)."""
    n = len(x) // feat.HOP
    if n == 0:
        return -100.0
    hops = x[: n * feat.HOP].reshape(n, feat.HOP).astype(np.float64)
    e = (hops**2).mean(axis=1)
    loud = e[e > 1e-5]
    return 10.0 * np.log10(loud.mean()) if loud.size else _rms_db(x)


def cut_event(x: np.ndarray, start_s: float, end_s: float, pad_s: float = 0.003) -> np.ndarray:
    """The event's samples plus a little context, with 2 ms raised-cosine fades."""
    a = max(0, int((start_s - pad_s) * feat.SR))
    b = min(len(x), int((end_s + pad_s) * feat.SR))
    ev = x[a:b].astype(np.float32).copy()
    f = min(len(ev) // 2, int(0.002 * feat.SR))
    if f > 0:
        ramp = 0.5 - 0.5 * np.cos(np.linspace(0, np.pi, f, dtype=np.float32))
        ev[:f] *= ramp
        ev[-f:] *= ramp[::-1]
    return ev


def mix(
    background: np.ndarray,
    regions: list[dict],
    bank: list[np.ndarray],
    cfg: DataConfig,
    rng: np.random.Generator,
    weights: np.ndarray | None = None,
) -> tuple[np.ndarray, list[dict]]:
    """Paste bank events (drawn with `weights`) into `background`; returns the mix and its
    regions (old + new)."""
    out = background.astype(np.float32).copy()
    regs = [dict(r) for r in regions]
    if not bank:
        return out, regs
    level = active_rms_db(background)
    n = int(rng.integers(cfg.events_per_mix[0], cfg.events_per_mix[1] + 1))
    for _ in range(n):
        ev = bank[int(rng.choice(len(bank), p=weights))]
        if rng.random() < 0.5:
            ev = -ev
        dur = len(ev) / feat.SR
        lo, hi = 0.6, len(out) / feat.SR - dur - 0.1
        if hi <= lo:
            break
        for _attempt in range(10):
            t = float(rng.uniform(lo, hi))
            if not any(t < r["end_s"] + 0.05 and r["start_s"] < t + dur + 0.05 for r in regs):
                break
        else:
            continue
        gain_db = level + float(rng.uniform(*cfg.esr_db)) - _rms_db(ev)
        a = int(t * feat.SR)
        out[a : a + len(ev)] += ev * np.float32(10 ** (gain_db / 20))
        regs.append({"start_s": t + 0.003, "end_s": t + dur - 0.003, "cls": "lip_smack"})
    return out, sorted(regs, key=lambda r: r["start_s"])


def tilt(x: np.ndarray, amount: float) -> np.ndarray:
    """A gentle EQ tilt: `amount` > 0 brightens, < 0 darkens (one-pole split near 2 kHz)."""
    a = np.exp(-2 * np.pi * 2000.0 / feat.SR)
    low = lfilter([1 - a], [1, -a], x).astype(np.float32)
    return (x + amount * (x - low)).astype(np.float32)


def align(ref: np.ndarray, y: np.ndarray, max_lag: int = 4096) -> np.ndarray:
    """`y` shifted to line up with `ref` (codecs add priming delay), same length as `ref`."""
    n = min(len(ref), len(y))
    size = 1 << int(np.ceil(np.log2(2 * n)))
    c = np.fft.irfft(np.fft.rfft(y[:n], size) * np.conj(np.fft.rfft(ref[:n], size)), size)
    lags = np.concatenate([c[: max_lag + 1], c[-max_lag:]])
    lag = int(np.argmax(lags))
    lag = lag if lag <= max_lag else lag - (2 * max_lag + 1)
    out = np.zeros_like(ref)
    src = y[lag:] if lag >= 0 else np.concatenate([np.zeros(-lag, dtype=y.dtype), y])
    m = min(len(out), len(src))
    out[:m] = src[:m]
    return out


def codec_roundtrip(x: np.ndarray, codec: str, kbps: int, ffmpeg: str) -> np.ndarray:
    """Encode and decode through ffmpeg (Opus or AAC at 48 kHz), realigned to the input."""
    with tempfile.TemporaryDirectory() as d:
        src, enc, dec = Path(d, "in.wav"), Path(d, "enc." + ("ogg" if codec == "opus" else "m4a")), Path(d, "out.wav")
        import soundfile as sf

        sf.write(src, feat.resample(x, feat.SR, 48_000), 48_000, subtype="FLOAT")
        lib = "libopus" if codec == "opus" else "aac"
        run = [ffmpeg, "-hide_banner", "-loglevel", "error", "-y"]
        subprocess.run([*run, "-i", src, "-c:a", lib, "-b:a", f"{kbps}k", enc], check=True)
        subprocess.run([*run, "-i", enc, "-ar", "48000", "-ac", "1", dec], check=True)
        y = feat.load_audio(dec)
    return align(x, y)


# ── building ────────────────────────────────────────────────────────────────────────────────


def _item(name, kind, split, x, regions, rejected, cfg, auto=False, role="events") -> Item:
    feats = feat.log_mel(x)
    y, w = frame_targets(len(feats), regions, rejected, cfg.hard_negative_weight)
    return Item(
        name=name,
        kind=kind,
        split=split,
        feats=feats,
        y=y,
        w=w,
        labels=labels_from_clip(regions, rejected),
        seconds=len(x) / feat.SR,
        auto=auto,
        role=role,
    )


def synthetic(hearo: list[str], seed: int, secs: float, workdir: Path) -> list[tuple[str, np.ndarray, list[dict]]]:
    """`hearo gen-signals` at `SR`: (role, audio, regions) for smacks, clicks, speech, music."""
    out = workdir / f"seed{seed}"
    subprocess.run(
        [*hearo, "gen-signals", str(out), "--sr", str(feat.SR), "--secs", str(secs), "--seed", str(seed)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    man = json.loads((out / "manifest.json").read_text())
    sr = man["sample_rate"]
    res = []
    for s in man["signals"]:
        role = {
            "speech_smacks.wav": "events",
            "speech_clicks.wav": "events",
            "speech.wav": "speech",
            "music.wav": "music",
        }
        if s["file"] not in role:
            continue
        cls = "mouth_click" if "clicks" in s["file"] else "lip_smack"
        regions = [{"start_s": e["start"] / sr, "end_s": (e["start"] + e["len"]) / sr, "cls": cls} for e in s["events"]]
        res.append((role[s["file"]] + ":" + s["file"], feat.load_audio(out / s["file"]), regions))
    return res


def cache_key(manifest_sha: str, cfg: DataConfig, codecs: bool = True, synthetic: bool = True) -> str:
    """Changes with anything that changes the sets, including whether ffmpeg (codec
    round-trips) and the `hearo` CLI (synthetic signals) were there to use."""
    blob = json.dumps(
        {
            "format": FORMAT,
            "manifest": manifest_sha,
            "cfg": asdict(cfg),
            "codecs": codecs and bool(cfg.codecs),
            "synthetic": synthetic,
        },
        sort_keys=True,
    )
    return hashlib.sha256(blob.encode()).hexdigest()[:16]


def build(
    manifest: Path,
    out_root: Path,
    cfg: DataConfig,
    hearo: list[str] | None,
    ffmpeg: str | None,
    log=print,
) -> Path:
    """Build (or reuse) the sets for `manifest`; returns the cache folder with `<split>.pkl`."""
    meta_path = manifest.parent / "meta.json"
    sha = json.loads(meta_path.read_text())["manifest_sha256"] if meta_path.exists() else _sha(manifest)
    key = cache_key(sha, cfg, codecs=ffmpeg is not None, synthetic=hearo is not None)
    folder = out_root / key
    if (folder / "done.json").exists():
        log(f"✔ data cache {folder} is current")
        return folder
    rng = np.random.default_rng(cfg.seed)
    clips = [json.loads(line) for line in manifest.read_text().splitlines() if line.strip()]
    sets: dict[str, list[Item]] = {"train": [], "val": [], "test": [], "synthetic_test": []}
    audio: dict[str, np.ndarray] = {}
    for c in clips:
        audio[c["stem"]] = feat.load_audio(Path(c["wav"]))
        sets[c["split"]].append(
            _item(c["stem"], "real", c["split"], audio[c["stem"]], c["regions"], c["rejected"], cfg, auto=c["auto"])
        )
    bank = [
        cut_event(audio[c["stem"]], r["start_s"], r["end_s"])
        for c in clips
        if c["split"] == "train"
        for r in c["regions"]
        if r["cls"] in TARGET_CLASSES
    ]
    # pool.jsonl (isolated train-day events) is a subset of these, so the bank takes every
    # train-day region instead: the pool alone is too small to mix with yet (2 events).
    with tempfile.TemporaryDirectory() as tmp:
        synth_train, synth_test = [], []
        if hearo:
            for seed in cfg.synthetic_train_seeds:
                synth_train += [
                    (f"synth{seed}:{n}", x, r) for n, x, r in synthetic(hearo, seed, cfg.synthetic_secs, Path(tmp))
                ]
            for seed in cfg.synthetic_test_seeds:
                synth_test += [
                    (f"synth{seed}:{n}", x, r) for n, x, r in synthetic(hearo, seed, cfg.synthetic_secs, Path(tmp))
                ]
        # Synthetic events join the bank too (more shapes than the few real ones so far), but
        # real events are drawn `real_event_share` of the time however many synthetic ones there are.
        synth_bank = [cut_event(x, r["start_s"], r["end_s"]) for _, x, regs in synth_train for r in regs]
    n_real = len(bank)
    bank += synth_bank
    weights = None
    if n_real and synth_bank:
        share = cfg.real_event_share
        weights = np.array([share / n_real] * n_real + [(1 - share) / len(synth_bank)] * len(synth_bank))
    log(f"event bank: {n_real} real + {len(synth_bank)} synthetic events")
    train_real = [c for c in clips if c["split"] == "train"]
    codec_p = np.array([p for _, _, p in cfg.codecs] + [max(0.0, 1 - sum(p for _, _, p in cfg.codecs))])
    for c in train_real:
        for k in range(cfg.mixes_per_clip):
            x, regs = mix(audio[c["stem"]], c["regions"], bank, cfg, rng, weights)
            x = tilt(x, float(rng.uniform(-0.5, 1.0)))
            x *= np.float32(10 ** (rng.uniform(*cfg.gain_db) / 20))
            pick = int(rng.choice(len(codec_p), p=codec_p / codec_p.sum()))
            if pick < len(cfg.codecs) and ffmpeg:
                name, kbps, _ = cfg.codecs[pick]
                x = codec_roundtrip(x, name, kbps, ffmpeg)
            peak = float(np.abs(x).max())
            if peak > 0.99:
                x *= np.float32(0.99 / peak)
            sets["train"].append(_item(f"{c['stem']}~mix{k}", "mix", "train", x, regs, c["rejected"], cfg))
    for name, x, regs in synth_train:
        sets["train"].append(_item(name, "synthetic", "train", x, regs, [], cfg, role=name.rsplit(":", 1)[1]))
    for name, x, regs in synth_test:
        sets["synthetic_test"].append(
            _item(name, "synthetic", "synthetic_test", x, regs, [], cfg, role=name.rsplit(":", 1)[1])
        )
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    for split, items in sets.items():
        with (folder / f"{split}.pkl").open("wb") as f:
            # Plain dicts and arrays: a pickled dataclass would name the module that wrote it
            # (`__main__` under `python -m hearo_ml.dataset`) and fail to load elsewhere.
            pickle.dump([_plain(i) for i in items], f)
    summary = {
        "key": key,
        "manifest_sha256": sha,
        "config": asdict(cfg),
        "codecs": ffmpeg is not None,
        "synthetic": hearo is not None,
        "bank_events": len(bank),
        "sets": {s: {"clips": len(v), "minutes": round(sum(i.seconds for i in v) / 60, 1)} for s, v in sets.items()},
    }
    (folder / "done.json").write_text(json.dumps(summary, indent=2) + "\n")
    log(json.dumps(summary["sets"]))
    return folder


def _plain(i: Item) -> dict:
    d = {k: getattr(i, k) for k in Item.__dataclass_fields__}
    d["labels"] = {k: [(s.start, s.end) for s in getattr(i.labels, k)] for k in ("targets", "neutral", "rejected")}
    return d


def _item_from(d: dict) -> Item:
    labels = Labels(**{k: tuple(Span(a, b) for a, b in v) for k, v in d["labels"].items()})
    return Item(**{**d, "labels": labels})


def load(folder: Path, split: str) -> list[Item]:
    with (folder / f"{split}.pkl").open("rb") as f:
        return [_item_from(d) for d in pickle.load(f)]  # our own cache, written by `build`


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def default_hearo() -> list[str]:
    """The release CLI through cargo (built once, then fast)."""
    return [
        "cargo",
        "run",
        "--quiet",
        "--release",
        "--manifest-path",
        str(REPO / "Cargo.toml"),
        "-p",
        "hearo-cli",
        "--",
    ]


def config_from_dict(d: dict) -> DataConfig:
    tuples = {
        k: tuple(tuple(x) if isinstance(x, list) else x for x in v) if isinstance(v, list) else v for k, v in d.items()
    }
    return DataConfig(**tuples)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="hearo_ml.dataset", description=__doc__.splitlines()[0])
    ap.add_argument("--manifest", type=Path, default=ML_DIR / "data/processed/snapshots/manifest.jsonl")
    ap.add_argument("--out", type=Path, default=ML_DIR / "data/processed/features")
    ap.add_argument("--config", type=Path, default=ML_DIR / "configs/detector.yaml", help="its `data` section")
    ap.add_argument("--no-codecs", action="store_true", help="skip ffmpeg round-trips")
    args = ap.parse_args(argv)
    import yaml

    cfg = config_from_dict((yaml.safe_load(args.config.read_text()) or {}).get("data", {}))
    if not args.manifest.exists():
        print(f"✖ {args.manifest} missing: run `just ml-import-snapshots` first", file=sys.stderr)
        return 1
    ffmpeg = None if args.no_codecs else shutil.which("ffmpeg")
    if ffmpeg is None and not args.no_codecs:
        print("⚠ ffmpeg not found: training mixes skip the codec round-trips", file=sys.stderr)
    build(args.manifest, args.out, cfg, default_hearo(), ffmpeg)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
