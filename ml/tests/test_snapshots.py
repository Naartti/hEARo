"""The snapshot importer: reviewed clips → manifest with by-day splits and a mixing pool."""

from __future__ import annotations

import datetime as dt
import json
import math
import struct
from pathlib import Path

import pytest

from hearo_ml import snapshots as snap

UTC = dt.UTC
SR = 48_000


def write_wav(path: Path, mono: list[float], channels: int = 2) -> None:
    """32-bit float WAV, like hEARo's snapshots."""
    data = b"".join(struct.pack("<f", v) * channels for v in mono)
    fmt = struct.pack("<HHIIHH", 3, channels, SR, SR * 4 * channels, 4 * channels, 32)
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
    path.write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)


def clip_audio(seconds: float = 1.0, events: tuple[tuple[float, float, float], ...] = ()) -> list[float]:
    """Quiet noise floor (-80 dB) with loud bursts at (start_s, end_s, amplitude)."""
    n = int(seconds * SR)
    x = [1e-4 * math.sin(i * 1.7) for i in range(n)]
    for start, end, amp in events:
        for i in range(int(start * SR), int(end * SR)):
            x[i] = amp * (1 if i % 2 else -1)
    return x


def add_clip(
    folder: Path,
    unix: int,
    label: str = "missed",
    reviewed: bool = True,
    regions: list[dict] | None = None,
    rejected: list[dict] | None = None,
    audio: list[float] | None = None,
) -> str:
    stem = f"snapshot-{unix}-{label}"
    write_wav(folder / f"{stem}.wav", audio if audio is not None else clip_audio())
    side = {"label": label, "unix_time": unix, "seconds": 1.0, "sample_rate": SR, "reviewed": reviewed}
    side["regions"] = regions or []
    if rejected:
        side["rejected"] = rejected
    (folder / f"{stem}.json").write_text(json.dumps(side))
    return stem


def day_unix(day: str, hour: int = 12) -> int:
    return int(dt.datetime.fromisoformat(f"{day}T{hour:02d}:00:00+00:00").timestamp())


def smack(start: float, end: float, cls: str = "lip_smack") -> dict:
    return {"start_s": start, "end_s": end, "class": cls}


def test_only_reviewed_clips_are_kept(tmp_path: Path) -> None:
    add_clip(tmp_path, day_unix("2026-11-13"), regions=[smack(0.5, 0.52)])
    add_clip(tmp_path, day_unix("2026-11-13", 13), reviewed=False)
    (tmp_path / "snapshot-5-missed.json").write_text("{ broken")
    clips, skipped = snap.read_clips(tmp_path, UTC)
    assert [c.day for c in clips] == ["2026-11-13"]
    assert clips[0].regions == [snap.Event(0.5, 0.52, "lip_smack")]
    reasons = dict(skipped)
    assert "not reviewed" in reasons.values()
    assert any("unreadable" in why for why in reasons.values())


def test_splits_are_by_day_and_stable() -> None:
    days = [f"2026-11-{d:02d}" for d in range(1, 31)]
    split = snap.assign_splits(days)
    assert set(split.values()) == {"train", "val", "test"}
    # Adding days never moves an existing (hashed) day: 30 days already have test and val.
    more = snap.assign_splits([*days, "2026-12-01", "2026-12-02"])
    assert all(more[d] == split[d] for d in days)
    # A handful of days still gets a test day (and val from five on).
    few = snap.assign_splits(["2026-11-13", "2026-11-14", "2026-11-15"])
    assert "test" in few.values()
    assert "val" in snap.assign_splits([f"2026-11-1{d}" for d in range(5)]).values()


def test_leakage_check_rejects_a_day_in_two_splits_and_auto_test_clips(tmp_path: Path) -> None:
    def clip(stem: str, day: str, split: str, auto: bool = False) -> snap.Clip:
        c = snap.Clip(stem, tmp_path / f"{stem}.wav", day, 0, "auto" if auto else "missed", auto, 8.0, SR)
        c.split = split
        return c

    snap.check_no_leakage([clip("a", "2026-11-13", "train"), clip("b", "2026-11-13", "train")])
    with pytest.raises(ValueError, match="both"):
        snap.check_no_leakage([clip("a", "2026-11-13", "train"), clip("b", "2026-11-13", "test")])
    with pytest.raises(ValueError, match="auto"):
        snap.check_no_leakage([clip("a", "2026-11-13", "test", auto=True)])


def test_auto_clips_on_test_days_are_left_out(tmp_path: Path) -> None:
    days = [f"2026-11-{d:02d}" for d in range(1, 11)]
    split = snap.assign_splits(days)
    test_day = next(d for d in days if split[d] == "test")
    add_clip(tmp_path, day_unix(test_day, 10), label="missed", regions=[smack(0.5, 0.52)])
    add_clip(tmp_path, day_unix(test_day, 11), label="auto", regions=[smack(0.5, 0.52)])
    for d in days:
        if d != test_day:
            add_clip(tmp_path, day_unix(d), regions=[smack(0.5, 0.52)])
    clips, _ = snap.read_clips(tmp_path, UTC)
    kept, dropped = snap.split_clips(clips)
    assert [why for _, why in dropped] == ["auto-saved clip on a test day"]
    assert all(not (c.split == "test" and c.auto) for c in kept)
    assert {c.day for c in kept if c.split == "test"} >= {test_day}


def test_read_wav_mono_float_and_int16(tmp_path: Path) -> None:
    write_wav(tmp_path / "f.wav", [0.5, -0.25])
    x, rate = snap.read_wav_mono(tmp_path / "f.wav")
    assert rate == SR and list(x) == [0.5, -0.25]
    pcm = struct.pack("<hh", 16384, -16384)  # one stereo frame: +0.5 / -0.5 → mono 0
    fmt = struct.pack("<HHIIHH", 1, 2, SR, SR * 4, 4, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(pcm)) + pcm
    (tmp_path / "i.wav").write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
    x, _ = snap.read_wav_mono(tmp_path / "i.wav")
    assert list(x) == [0.0]
    (tmp_path / "bad.wav").write_bytes(b"nope")
    with pytest.raises(ValueError):
        snap.read_wav_mono(tmp_path / "bad.wav")


def test_pool_takes_isolated_quiet_events_from_training_clips_only(tmp_path: Path) -> None:
    audio = clip_audio(1.0, ((0.30, 0.31, 0.5), (0.60, 0.61, 0.5), (0.64, 0.65, 0.5), (0.85, 0.86, 0.5)))
    c = snap.Clip("s", tmp_path / "s.wav", "2026-11-13", 0, "missed", False, 1.0, SR)
    c.regions = [
        snap.Event(0.30, 0.31, "lip_smack"),  # isolated, quiet around: in the pool
        snap.Event(0.60, 0.61, "lip_smack"),  # 30 ms from the next one: not isolated
        snap.Event(0.64, 0.65, "mouth_click"),
        snap.Event(0.85, 0.86, "breath"),  # not a pool class
    ]
    c.split = "train"
    pool = snap.pool_events(c, *snap.read_wav_mono_from(audio, SR))
    assert [(p["start_s"], p["cls"]) for p in pool] == [(0.30, "lip_smack")]
    assert pool[0]["event_db"] - pool[0]["context_db"] >= snap.POOL_QUIET_DB
    c.split = "test"
    assert snap.pool_events(c, *snap.read_wav_mono_from(audio, SR)) == []


def test_pool_skips_events_without_quiet_context(tmp_path: Path) -> None:
    # A smack with loud audio right after it (speech starting): not a clean pool event.
    audio = clip_audio(1.0, ((0.30, 0.31, 0.5), (0.31, 0.40, 0.2)))
    c = snap.Clip("s", tmp_path / "s.wav", "2026-11-13", 0, "missed", False, 1.0, SR)
    c.regions = [snap.Event(0.30, 0.31, "lip_smack")]
    c.split = "train"
    assert snap.pool_events(c, *snap.read_wav_mono_from(audio, SR)) == []


def test_write_end_to_end(tmp_path: Path) -> None:
    folder, out = tmp_path / "snaps", tmp_path / "out"
    folder.mkdir()
    days = [f"2026-11-{d:02d}" for d in range(1, 11)]
    audio = clip_audio(1.0, ((0.30, 0.31, 0.5),))
    for i, d in enumerate(days):
        add_clip(folder, day_unix(d), regions=[smack(0.30, 0.31)], audio=audio)
        add_clip(
            folder,
            day_unix(d, 13),
            label="auto",
            regions=[smack(0.30, 0.31)],
            rejected=[{"start_s": 0.7, "end_s": 0.71, "module": "desmack"}],
            audio=audio,
        )
        if i == 0:
            add_clip(folder, day_unix(d, 14), reviewed=False)
    summary = snap.write(folder, out, UTC)
    lines = [json.loads(line) for line in (out / "manifest.jsonl").read_text().splitlines()]
    assert summary["clips"] == len(lines)
    assert {line["split"] for line in lines} == {"train", "val", "test"}
    by_day: dict[str, set[str]] = {}
    for line in lines:
        by_day.setdefault(line["day"], set()).add(line["split"])
    assert all(len(s) == 1 for s in by_day.values()), "a day never spans splits"
    assert not any(line["auto"] and line["split"] == "test" for line in lines)
    pool = [json.loads(line) for line in (out / "pool.jsonl").read_text().splitlines()]
    train_days = {line["day"] for line in lines if line["split"] == "train"}
    assert pool and all(p["day"] in train_days for p in pool)
    assert summary["train"]["hard_negatives"] > 0
    assert summary["skipped_by_reason"]["not reviewed"] == 1
    assert summary["skipped_by_reason"]["auto-saved clip on a test day"] >= 1
    assert summary["skipped"] == sum(summary["skipped_by_reason"].values())


def test_main_prints_a_summary(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    folder = tmp_path / "snaps"
    folder.mkdir()
    add_clip(folder, day_unix("2026-11-13"), regions=[smack(0.3, 0.31)])
    assert snap.main(["--snapshots", str(folder), "--out", str(tmp_path / "out")]) == 0
    assert '"clips": 1' in capsys.readouterr().out
