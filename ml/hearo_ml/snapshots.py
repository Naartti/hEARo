"""Training manifest from the human's reviewed hotkey snapshots (ADR 0017, docs/ml/README.md).

`python -m hearo_ml.snapshots` (or `just ml-import-snapshots`) reads every snapshot sidecar in
`~/Library/Application Support/hEARo/snapshots/`, keeps the *reviewed* (exhaustively labelled)
clips and writes, to a git-ignored folder:

- `manifest.jsonl` — one line per usable clip: its WAV path, recording day, split, regions
  (events), rejected spans (hard negatives) and label;
- `pool.jsonl` — clean, isolated events from *training* days for synthetic mixing.

Rules (docs/ml/README.md):

- **Splits by recording day**, never by clip: the same people recur across a day's meetings.
  A *day* is the day a listening session started: clips less than `SESSION_GAP_S` apart are one
  session, so a call across midnight stays on one side. `check_no_leakage` enforces that no
  day spans two splits and that pool events come from training days; `write` runs it.
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
#: Clips closer together than this belong to one listening session (and so one "day").
SESSION_GAP_S = 3600

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


def _events(items: object, default_cls: str, seconds: float = 0.0) -> list[Event]:
    """Valid spans only: finite, 0 ≤ start < end, and inside the clip when its length is known
    (anything else is a hand edit gone wrong, and mustn't become a training label)."""
    out = []
    for r in items if isinstance(items, list) else []:
        try:
            e = Event(float(r["start_s"]), float(r["end_s"]), str(r.get("class", default_cls)))
        except (KeyError, TypeError, ValueError, AttributeError):
            continue
        inside = seconds <= 0 or e.end_s <= seconds + 1e-3
        if math.isfinite(e.start_s) and math.isfinite(e.end_s) and 0 <= e.start_s < e.end_s and inside:
            out.append(e)
    return sorted(out, key=lambda e: e.start_s)


def _stem_parts(stem: str) -> list[str]:
    # snapshot-<unix>-<label>[-n][-importedN]
    return stem.split("-")


def _unix_from_stem(stem: str) -> int | None:
    parts = _stem_parts(stem)
    return int(parts[1]) if len(parts) > 2 and parts[1].isdigit() else None


def is_auto(stem: str, label: str) -> bool:
    """An auto-saved clip, by its sidecar label or (like the engine's `is_auto`) its name."""
    parts = _stem_parts(stem)
    return label == "auto" or (len(parts) > 2 and parts[2] == "auto")


def _clip(side_path: Path, side: dict) -> Clip | str:
    """A clip from a parsed sidecar, or why it can't be used."""
    stem = side_path.stem
    wav = side_path.with_suffix(".wav")
    if side.get("reviewed") is not True:
        return "not reviewed"
    if not wav.exists():
        return "WAV missing"
    unix = side.get("unix_time")
    unix = unix if isinstance(unix, int) and not isinstance(unix, bool) else _unix_from_stem(stem)
    if unix is None:
        return "no time"
    label = str(side.get("label") or "")
    seconds = float(side.get("seconds") or 0.0)
    return Clip(
        stem=stem,
        wav=wav,
        day="",
        unix_time=unix,
        label=label,
        auto=is_auto(stem, label),
        seconds=seconds,
        sample_rate=float(side.get("sample_rate") or 0.0),
        regions=_events(side.get("regions"), "lip_smack", seconds),
        rejected=_events(side.get("rejected"), "rejected", seconds),
    )


def assign_days(clips: list[Clip], tz: dt.tzinfo | None = None) -> None:
    """Set each clip's `day`: the local date its listening session started (clips sorted)."""
    session_day, last = "", None
    for c in clips:
        if last is None or c.unix_time - last >= SESSION_GAP_S:
            session_day = dt.datetime.fromtimestamp(c.unix_time, tz).date().isoformat()
        c.day = session_day
        last = c.unix_time


def read_clips(folder: Path, tz: dt.tzinfo | None = None) -> tuple[list[Clip], list[tuple[str, str]]]:
    """Reviewed clips in `folder` (sorted by time, `day` set) and the skipped ones as (stem, why).

    `tz` sets what a "day" is (default: this Mac's local time zone, as the human lives it; pass
    a fixed one to make a manifest reproducible elsewhere).
    """
    clips: list[Clip] = []
    skipped: list[tuple[str, str]] = []
    for side_path in sorted(folder.glob("snapshot-*.json")):
        try:
            side = json.loads(side_path.read_text())
            if not isinstance(side, dict):
                raise TypeError("not an object")
            clip = _clip(side_path, side)
        except (OSError, ValueError, TypeError) as e:
            skipped.append((side_path.stem, f"unreadable sidecar ({e})"))
            continue
        if isinstance(clip, str):
            skipped.append((side_path.stem, clip))
        else:
            clips.append(clip)
    clips.sort(key=lambda c: (c.unix_time, c.stem))
    assign_days(clips, tz)
    return clips, skipped


def _day_hash(day: str) -> float:
    """A stable number in [0, 1) per day: a day keeps its split as more days arrive."""
    return int.from_bytes(hashlib.sha256(day.encode()).digest()[:8], "big") / 2**64


def assign_splits(
    days: Iterable[str], locked: dict[str, str] | None = None, auto_clips: dict[str, int] | None = None
) -> dict[str, str]:
    """Split per recording day.

    A day in `locked` (an earlier manifest's `splits.lock.json`) keeps its split forever, so a
    test day is never later trained or tuned on. A new day gets its split from a stable hash
    (≈ `TEST_SHARE` test, `VAL_SHARE` validation). Fallbacks so a small set is still usable,
    applied to new days only: with two or more days at least one is train, with three or more
    one is test, with five or more one is validation — taken from the split with the most new
    days. A fallback test day is the new day with the fewest auto-saved clips (they can't be
    test data, so a test day full of them wastes them), then the lowest hash.
    """
    wanted = set(days)
    locked = {d: s for d, s in (locked or {}).items() if s in SPLITS}
    autos = auto_clips or {}
    uniq = sorted(wanted | set(locked))
    split = dict(locked)
    for d in uniq:
        if d not in split:
            h = _day_hash(d)
            split[d] = "test" if h < TEST_SHARE else "val" if h < TEST_SHARE + VAL_SHARE else "train"
    pick = {
        "train": lambda ds: max(ds, key=_day_hash),
        "test": lambda ds: min(ds, key=lambda d: (autos.get(d, 0), _day_hash(d))),
        "val": lambda ds: min(ds, key=_day_hash),
    }
    for need, min_days in (("train", 2), ("test", 3), ("val", 5)):
        if len(uniq) < min_days or need in split.values():
            continue
        movable: dict[str, list[str]] = {}
        for d in uniq:
            if d not in locked:
                movable.setdefault(split[d], []).append(d)
        # Never empty a split to fill another.
        donors = [s for s in movable if s != need and list(split.values()).count(s) > 1]
        if donors:
            donor = max(donors, key=lambda s: len(movable[s]))
            split[pick[need](movable[donor])] = need
    return {d: split[d] for d in uniq if d in wanted}


def check_no_leakage(clips: Iterable[Clip], pool: Iterable[dict] = ()) -> None:
    """Raise if a day appears in two splits, an auto clip is test data, or a pool event comes
    from a day that isn't training."""
    seen: dict[str, str] = {}
    for c in clips:
        if c.split not in SPLITS:
            raise ValueError(f"{c.stem}: no split")
        if seen.setdefault(c.day, c.split) != c.split:
            raise ValueError(f"day {c.day} is in both {seen[c.day]} and {c.split} ({c.stem})")
        if c.split == "test" and c.auto:
            raise ValueError(f"{c.stem}: auto-saved clips can't be test data (ADR 0017)")
    for p in pool:
        if seen.get(p["day"]) != "train":
            raise ValueError(f"pool event from {p['stem']} is on a {seen.get(p['day'])} day")


def split_clips(clips: list[Clip], locked: dict[str, str] | None = None) -> tuple[list[Clip], list[tuple[str, str]]]:
    """Give every clip its day's split; auto clips on test days are left out (returned)."""
    autos: dict[str, int] = {}
    for c in clips:
        autos[c.day] = autos.get(c.day, 0) + int(c.auto)
    split = assign_splits((c.day for c in clips), locked, autos)
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
    pos, fmt, raw, fmt_body = 12, None, None, b""
    try:
        while pos + 8 <= len(data):
            cid, size = data[pos : pos + 4], struct.unpack_from("<I", data, pos + 4)[0]
            body = data[pos + 8 : pos + 8 + size]
            if cid == b"fmt ":
                fmt, fmt_body = struct.unpack_from("<HHIIHH", body), body
            elif cid == b"data":
                raw = body
            pos += 8 + size + (size & 1)
    except struct.error as e:
        raise ValueError(f"{path.name}: truncated header ({e})") from e
    if fmt is None or raw is None:
        raise ValueError(f"{path.name}: no fmt or data chunk")
    tag, channels, rate, _, _, bits = fmt
    if tag == 0xFFFE:
        # WAVE_FORMAT_EXTENSIBLE (what hound writes for 32-bit): the real format is the first two
        # bytes of the sub-format GUID, 24 bytes into the fmt body.
        if len(fmt_body) < 26:
            raise ValueError(f"{path.name}: truncated extensible header")
        tag = struct.unpack_from("<H", fmt_body, 24)[0]
    if tag == 3 and bits == 32:
        samples, scale = array("f"), 1.0
        samples.frombytes(raw[: len(raw) // 4 * 4])
    elif tag == 1 and bits == 16:
        samples, scale = array("h"), 1 / 32768
        samples.frombytes(raw[: len(raw) // 2 * 2])
    else:
        raise ValueError(f"{path.name}: unsupported format {tag}/{bits} bit")
    if sys.byteorder != "little":
        samples.byteswap()
    channels = max(channels, 1)
    n = len(samples) // channels
    mono = array("f", [0.0]) * n
    for c in range(channels):
        for i, v in enumerate(samples[c : n * channels : channels]):
            mono[i] += v
    k = scale / channels
    return array("f", (v * k for v in mono)), float(rate)


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
    spans = clip.regions + clip.rejected
    for k, e in enumerate(clip.regions):
        if e.cls not in POOL_CLASSES:
            continue
        # By index, not value: a region marked twice is two regions, so neither is isolated.
        if any(
            j != k and o.start_s < e.end_s + POOL_ISOLATION_S and e.start_s < o.end_s + POOL_ISOLATION_S
            for j, o in enumerate(spans)
        ):
            continue
        s, t = int(e.start_s * rate), int(e.end_s * rate)
        ctx = int(POOL_CONTEXT_S * rate)
        if t <= s or s - ctx < 0 or t + ctx > len(audio):
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


#: Fewer test events than this: metrics have intervals too wide to compare models on.
MIN_TEST_EVENTS = 50
LOCK = "splits.lock.json"


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(folder: Path, out: Path, tz: dt.tzinfo | None = None) -> dict:
    """Write `manifest.jsonl`, `pool.jsonl`, `meta.json` and the split lock to `out`; return a
    summary. Days already in `out/splits.lock.json` keep their split."""
    lock_path = out / LOCK
    try:
        locked = json.loads(lock_path.read_text()) if lock_path.exists() else {}
    except ValueError as e:
        raise ValueError(f"{lock_path}: unreadable split lock ({e}); fix or delete it on purpose") from e
    clips, skipped = read_clips(folder, tz)
    kept, dropped = split_clips(clips, locked)
    pool: list[dict] = []
    usable: list[Clip] = []
    for c in kept:
        if c.split == "train" and any(e.cls in POOL_CLASSES for e in c.regions):
            try:
                audio, rate = read_wav_mono(c.wav)
            except (OSError, ValueError) as e:
                # A clip whose audio can't be read is no use for training either.
                skipped.append((c.stem, f"unreadable WAV ({e})"))
                continue
            pool.extend(pool_events(c, audio, rate))
        usable.append(c)
    check_no_leakage(usable, pool)
    out.mkdir(parents=True, exist_ok=True)
    (out / "manifest.jsonl").write_text("".join(json.dumps(_clip_json(c)) + "\n" for c in usable))
    (out / "pool.jsonl").write_text("".join(json.dumps(p) + "\n" for p in pool))
    days = {**locked, **{c.day: c.split for c in clips}}
    lock_path.write_text(json.dumps(dict(sorted(days.items())), indent=2) + "\n")
    meta = {
        "manifest_sha256": _sha256(out / "manifest.jsonl"),
        "pool_sha256": _sha256(out / "pool.jsonl"),
        "tz": str(tz) if tz else "local",
        "params": {
            "TEST_SHARE": TEST_SHARE,
            "VAL_SHARE": VAL_SHARE,
            "SESSION_GAP_S": SESSION_GAP_S,
            "POOL_ISOLATION_S": POOL_ISOLATION_S,
            "POOL_CONTEXT_S": POOL_CONTEXT_S,
            "POOL_QUIET_DB": POOL_QUIET_DB,
        },
        "splits": dict(sorted(days.items())),
    }
    (out / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    reasons: dict[str, int] = {}
    for _, why in skipped + dropped:
        key = why.split(" (")[0]
        reasons[key] = reasons.get(key, 0) + 1
    summary: dict = {
        "clips": len(usable),
        "skipped": len(skipped) + len(dropped),
        "skipped_by_reason": reasons,
        "pool": len(pool),
    }
    for s in SPLITS:
        in_s = [c for c in usable if c.split == s]
        summary[s] = {
            "days": len({c.day for c in in_s}),
            "clips": len(in_s),
            "events": sum(len(c.regions) for c in in_s),
            "hard_negatives": sum(len(c.rejected) for c in in_s),
        }
    warnings = []
    if summary["test"]["events"] < MIN_TEST_EVENTS:
        warnings.append(
            f"test has {summary['test']['events']} events (< {MIN_TEST_EVENTS}): metrics are rough, "
            "don't compare models on them yet"
        )
    if summary["val"]["clips"] == 0:
        warnings.append("no validation days yet: don't tune on test — wait for more days")
    summary["warnings"] = warnings
    summary["manifest_sha256"] = meta["manifest_sha256"]
    return summary


def default_folder() -> Path:
    return Path.home() / "Library" / "Application Support" / "hEARo" / "snapshots"


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="hearo_ml.snapshots", description=__doc__.splitlines()[0])
    p.add_argument("--snapshots", type=Path, default=default_folder(), help="snapshot folder")
    p.add_argument("--out", type=Path, default=Path("data/processed/snapshots"), help="output folder")
    p.add_argument("--tz", help="IANA time zone that defines a day (default: this Mac's local zone)")
    args = p.parse_args(argv)
    tz = None
    if args.tz:
        from zoneinfo import ZoneInfo

        tz = ZoneInfo(args.tz)
    summary = write(args.snapshots, args.out, tz)
    print(json.dumps(summary, indent=2))
    print(f"wrote {args.out / 'manifest.jsonl'} and {args.out / 'pool.jsonl'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
