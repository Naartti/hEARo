"""Training manifest from the human's reviewed hotkey snapshots (ADR 0017, docs/ml/README.md).

`python -m hearo_ml.snapshots` (or `just ml-import-snapshots`) reads every snapshot sidecar in
`~/Library/Application Support/hEARo/snapshots/`, keeps the *reviewed* (exhaustively labelled)
clips and writes, to a git-ignored folder:

- `manifest.jsonl` — one line per usable clip: its WAV path, recording day, split, regions
  (events), rejected spans (hard negatives) and label;
- `pool.jsonl` — clean, isolated events from *training* days for synthetic mixing.

Rules (docs/ml/README.md):

- **Splits by recording day**, never by clip: the same people recur across a day's meetings.
  `check_no_leakage` enforces that no day spans two splits; `write` runs it.
- The **test set** is real hotkey clips only: auto-saved clips only contain what the DSP
  de-smack already finds, so on test days they are left out (and not trained on either: same
  day).
- The **mixing pool** takes lip smacks and mouth clicks from training clips that have no other
  region close by and sit in quiet: the audio just before and after is well below the event.

Only the standard library: the WAVs are hEARo's own (32-bit float or 16-bit PCM, stereo).
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import struct
import sys
from array import array
from collections.abc import Iterable
from dataclasses import asdict, dataclass, field
from pathlib import Path

#: Classes the mixing pool takes (the transient mouth noises a detector learns first).
POOL_CLASSES = ("lip_smack", "mouth_click")
#: No other region within this much of a pool event.
POOL_ISOLATION_S = 0.1
#: Context checked for quiet on each side of a pool event.
POOL_CONTEXT_S = 0.05
#: The context's RMS must be at least this far below the event's RMS.
POOL_QUIET_DB = 20.0
#: Share of recording days held out for test / validation (by a stable hash of the day).
TEST_SHARE = 0.2
VAL_SHARE = 0.1

SPLITS = ("train", "val", "test")


@dataclass(frozen=True)
class Event:
    start_s: float
    end_s: float
    cls: str


@dataclass
class Clip:
    stem: str
    wav: Path
    day: str
    unix_time: int
    label: str
    auto: bool
    seconds: float
    sample_rate: float
    regions: list[Event] = field(default_factory=list)
    rejected: list[Event] = field(default_factory=list)
    split: str = ""


def _events(items: object, default_cls: str) -> list[Event]:
    out = []
    for r in items if isinstance(items, list) else []:
        try:
            out.append(Event(float(r["start_s"]), float(r["end_s"]), str(r.get("class", default_cls))))
        except (KeyError, TypeError, ValueError):
            continue
    return sorted(out, key=lambda e: e.start_s)


def _unix_from_stem(stem: str) -> int | None:
    # snapshot-<unix>-<label>[-n][-importedN]
    parts = stem.split("-")
    return int(parts[1]) if len(parts) > 2 and parts[1].isdigit() else None


def read_clips(folder: Path, tz: dt.tzinfo | None = None) -> tuple[list[Clip], list[tuple[str, str]]]:
    """Reviewed clips in `folder` (sorted by time) and the skipped ones as (stem, why).

    `tz` sets what a "day" is (default: this Mac's local time zone, as the human lives it).
    """
    clips: list[Clip] = []
    skipped: list[tuple[str, str]] = []
    for side_path in sorted(folder.glob("snapshot-*.json")):
        stem = side_path.stem
        wav = side_path.with_suffix(".wav")
        try:
            side = json.loads(side_path.read_text())
        except (OSError, ValueError) as e:
            skipped.append((stem, f"unreadable sidecar ({e})"))
            continue
        if side.get("reviewed") is not True:
            skipped.append((stem, "not reviewed"))
            continue
        if not wav.exists():
            skipped.append((stem, "WAV missing"))
            continue
        unix = side.get("unix_time") if isinstance(side.get("unix_time"), int) else _unix_from_stem(stem)
        if unix is None:
            skipped.append((stem, "no time"))
            continue
        label = str(side.get("label", ""))
        day = dt.datetime.fromtimestamp(unix, tz).astimezone(tz).date().isoformat()
        clips.append(
            Clip(
                stem=stem,
                wav=wav,
                day=day,
                unix_time=unix,
                label=label,
                auto=label == "auto",
                seconds=float(side.get("seconds", 0.0)),
                sample_rate=float(side.get("sample_rate", 0.0)),
                regions=_events(side.get("regions"), "lip_smack"),
                rejected=_events(side.get("rejected"), "rejected"),
            )
        )
    clips.sort(key=lambda c: (c.unix_time, c.stem))
    return clips, skipped


def _day_hash(day: str) -> float:
    """A stable number in [0, 1) per day: a day keeps its split as more days arrive."""
    return int.from_bytes(hashlib.sha256(day.encode()).digest()[:8], "big") / 2**64


def assign_splits(days: Iterable[str]) -> dict[str, str]:
    """Split per recording day: by a stable hash, so a day never moves when others are added.

    With three or more days, at least one is test (the lowest hash, if none hashed into test);
    with five or more, at least one is validation. Those fallbacks can move once a hashed day
    takes over; compare models only on the same manifest.
    """
    uniq = sorted(set(days))
    split = {}
    for d in uniq:
        h = _day_hash(d)
        split[d] = "test" if h < TEST_SHARE else "val" if h < TEST_SHARE + VAL_SHARE else "train"
    by_hash = sorted(uniq, key=_day_hash)
    if len(uniq) >= 3 and "test" not in split.values():
        split[by_hash[0]] = "test"
    if len(uniq) >= 5 and "val" not in split.values():
        split[next(d for d in by_hash if split[d] == "train")] = "val"
    return split


def check_no_leakage(clips: Iterable[Clip]) -> None:
    """Raise if a recording day appears in two splits, or an auto clip is in the test set."""
    seen: dict[str, str] = {}
    for c in clips:
        if c.split not in SPLITS:
            raise ValueError(f"{c.stem}: no split")
        if seen.setdefault(c.day, c.split) != c.split:
            raise ValueError(f"day {c.day} is in both {seen[c.day]} and {c.split} ({c.stem})")
        if c.split == "test" and c.auto:
            raise ValueError(f"{c.stem}: auto-saved clips can't be test data (ADR 0017)")


def split_clips(clips: list[Clip]) -> tuple[list[Clip], list[tuple[str, str]]]:
    """Give every clip its day's split; auto clips on test days are left out (returned)."""
    split = assign_splits(c.day for c in clips)
    kept, dropped = [], []
    for c in clips:
        c.split = split[c.day]
        if c.split == "test" and c.auto:
            dropped.append((c.stem, "auto-saved clip on a test day"))
        else:
            kept.append(c)
    check_no_leakage(kept)
    return kept, dropped


def read_wav_mono(path: Path) -> tuple[array, float]:
    """Mono samples (mean of the channels) and the sample rate of a float32 or int16 WAV."""
    data = path.read_bytes()
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError(f"{path.name}: not a WAV file")
    pos, fmt, raw = 12, None, None
    while pos + 8 <= len(data):
        cid, size = data[pos : pos + 4], struct.unpack_from("<I", data, pos + 4)[0]
        body = data[pos + 8 : pos + 8 + size]
        if cid == b"fmt ":
            fmt = struct.unpack_from("<HHIIHH", body)
        elif cid == b"data":
            raw = body
        pos += 8 + size + (size & 1)
    if fmt is None or raw is None:
        raise ValueError(f"{path.name}: no fmt or data chunk")
    tag, channels, rate, _, _, bits = fmt
    if tag in (3, 0xFFFE) and bits == 32:
        samples = array("f")
        samples.frombytes(raw[: len(raw) // 4 * 4])
        scale = 1.0
    elif tag in (1, 0xFFFE) and bits == 16:
        samples = array("h")
        samples.frombytes(raw[: len(raw) // 2 * 2])
        scale = 1 / 32768
    else:
        raise ValueError(f"{path.name}: unsupported format {tag}/{bits} bit")
    if sys.byteorder != "little":
        samples.byteswap()
    n = len(samples) // max(channels, 1)
    mono = array("f", (sum(samples[i * channels + c] for c in range(channels)) * scale / channels for i in range(n)))
    return mono, float(rate)


def read_wav_mono_from(samples: Iterable[float], rate: float) -> tuple[array, float]:
    """Mono samples already in memory, in `read_wav_mono`'s shape (tests, other loaders)."""
    return array("f", samples), float(rate)


def _rms_db(x: array, a: int, b: int) -> float:
    a, b = max(a, 0), min(b, len(x))
    if b <= a:
        return -math.inf
    p = sum(v * v for v in x[a:b]) / (b - a)
    return 10 * math.log10(p) if p > 0 else -math.inf


def pool_events(clip: Clip, audio: array, rate: float) -> list[dict]:
    """Isolated, quiet-context pool events of a training clip (empty for other splits)."""
    if clip.split != "train":
        return []
    out = []
    for e in clip.regions:
        if e.cls not in POOL_CLASSES:
            continue
        others = [o for o in clip.regions + clip.rejected if o != e]
        if any(o.start_s < e.end_s + POOL_ISOLATION_S and e.start_s < o.end_s + POOL_ISOLATION_S for o in others):
            continue
        s, t = int(e.start_s * rate), int(e.end_s * rate)
        ctx = int(POOL_CONTEXT_S * rate)
        if s - ctx < 0 or t + ctx > len(audio):
            continue
        ev_db = _rms_db(audio, s, t)
        ctx_db = max(_rms_db(audio, s - ctx, s), _rms_db(audio, t, t + ctx))
        if ev_db - ctx_db >= POOL_QUIET_DB:
            out.append({**asdict(e), "stem": clip.stem, "day": clip.day, "event_db": ev_db, "context_db": ctx_db})
    return out


def _clip_json(c: Clip) -> dict:
    d = asdict(c)
    d["wav"] = str(c.wav)
    return d


def write(folder: Path, out: Path, tz: dt.tzinfo | None = None) -> dict:
    """Write `manifest.jsonl` and `pool.jsonl` to `out`; return a summary."""
    clips, skipped = read_clips(folder, tz)
    kept, dropped = split_clips(clips)
    pool: list[dict] = []
    for c in kept:
        if c.split == "train" and any(e.cls in POOL_CLASSES for e in c.regions):
            try:
                audio, rate = read_wav_mono(c.wav)
            except (OSError, ValueError) as e:
                skipped.append((c.stem, f"pool: {e}"))
                continue
            pool.extend(pool_events(c, audio, rate))
    check_no_leakage(kept)
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.jsonl").write_text("".join(json.dumps(_clip_json(c)) + "\n" for c in kept))
    (out / "pool.jsonl").write_text("".join(json.dumps(p) + "\n" for p in pool))
    reasons: dict[str, int] = {}
    for _, why in skipped + dropped:
        key = why.split(" (")[0].split(":")[0]
        reasons[key] = reasons.get(key, 0) + 1
    summary: dict = {
        "clips": len(kept),
        "skipped": len(skipped) + len(dropped),
        "skipped_by_reason": reasons,
        "pool": len(pool),
    }
    for s in SPLITS:
        in_s = [c for c in kept if c.split == s]
        summary[s] = {
            "days": len({c.day for c in in_s}),
            "clips": len(in_s),
            "events": sum(len(c.regions) for c in in_s),
            "hard_negatives": sum(len(c.rejected) for c in in_s),
        }
    return summary


def default_folder() -> Path:
    return Path.home() / "Library" / "Application Support" / "hEARo" / "snapshots"


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="hearo_ml.snapshots", description=__doc__.splitlines()[0])
    p.add_argument("--snapshots", type=Path, default=default_folder(), help="snapshot folder")
    p.add_argument("--out", type=Path, default=Path("data/processed/snapshots"), help="output folder")
    args = p.parse_args(argv)
    summary = write(args.snapshots, args.out)
    print(json.dumps(summary, indent=2))
    print(f"wrote {args.out / 'manifest.jsonl'} and {args.out / 'pool.jsonl'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
