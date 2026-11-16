"""The snapshot importer: reviewed clips → manifest with by-day splits and a mixing pool."""

from __future__ import annotations

import datetime as dt
import json
import math
import struct
from array import array
from pathlib import Path

import pytest

from hearo_ml import snapshots as snap

UTC = dt.UTC
SR = 48_000


#: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT / _PCM, as in a WAVE_FORMAT_EXTENSIBLE header.
FLOAT_GUID = bytes.fromhex("0300000000001000800000aa00389b71")
PCM_GUID = bytes.fromhex("0100000000001000800000aa00389b71")


def wav_bytes(frames: list[tuple[float, ...]], bits: int = 32, guid: bytes = FLOAT_GUID) -> bytes:
    """A WAVE_FORMAT_EXTENSIBLE file, the header hound (hEARo's writer) uses for 32-bit audio."""
    channels = len(frames[0]) if frames else 2
    if guid == FLOAT_GUID:
        data = b"".join(struct.pack("<f", v) for f in frames for v in f)
    else:
        data = b"".join(struct.pack("<i" if bits == 32 else "<h", int(v)) for f in frames for v in f)
    block = bits // 8 * channels
    fmt = struct.pack("<HHIIHH", 0xFFFE, channels, SR, SR * block, block, bits)
    fmt += struct.pack("<HHI", 22, bits, 3) + guid
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", len(data)) + data
    return b"RIFF" + struct.pack("<I", len(body)) + body


def write_wav(path: Path, mono: list[float], channels: int = 2) -> None:
    """32-bit float stereo WAV, like hEARo's snapshots (the same signal in each channel)."""
    path.write_bytes(wav_bytes([(v,) * channels for v in mono]))


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
    # hound's extensible float header, channels that differ: L 0.5, R 0 → mono 0.25.
    (tmp_path / "f.wav").write_bytes(wav_bytes([(0.5, 0.0), (-0.25, -0.25)]))
    x, rate = snap.read_wav_mono(tmp_path / "f.wav")
    assert rate == SR and list(x) == [0.25, -0.25]
    # Plain 16-bit PCM (tag 1): (16384, 0) → (0.5 + 0) / 2.
    pcm = struct.pack("<hh", 16384, 0)
    fmt = struct.pack("<HHIIHH", 1, 2, SR, SR * 4, 4, 16)
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", len(pcm)) + pcm
    (tmp_path / "i.wav").write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
    x, _ = snap.read_wav_mono(tmp_path / "i.wav")
    assert list(x) == [0.25]
    # Extensible 32-bit *integer* PCM would read as float garbage: refused instead.
    (tmp_path / "i32.wav").write_bytes(wav_bytes([(1, 2)], bits=32, guid=PCM_GUID))
    with pytest.raises(ValueError, match="unsupported"):
        snap.read_wav_mono(tmp_path / "i32.wav")
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
    pool = snap.pool_events(c, array("f", audio), SR)
    assert [(p["start_s"], p["cls"]) for p in pool] == [(0.30, "lip_smack")]
    assert pool[0]["event_db"] - pool[0]["context_db"] >= snap.POOL_QUIET_DB
    c.split = "test"
    assert snap.pool_events(c, array("f", audio), SR) == []


def test_pool_skips_events_without_quiet_context(tmp_path: Path) -> None:
    # A smack with loud audio right after it (speech starting): not a clean pool event.
    audio = clip_audio(1.0, ((0.30, 0.31, 0.5), (0.31, 0.40, 0.2)))
    c = snap.Clip("s", tmp_path / "s.wav", "2026-11-13", 0, "missed", False, 1.0, SR)
    c.regions = [snap.Event(0.30, 0.31, "lip_smack")]
    c.split = "train"
    assert snap.pool_events(c, array("f", audio), SR) == []


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


def test_bad_sidecars_and_missing_files_are_skipped_not_fatal(tmp_path: Path) -> None:
    add_clip(tmp_path, day_unix("2026-11-13"), regions=[smack(0.5, 0.52)])
    (tmp_path / "snapshot-1-missed.json").write_text("[]")  # parses, but isn't an object
    (tmp_path / "snapshot-1-missed.wav").write_bytes(b"")
    (tmp_path / "snapshot-2-missed.json").write_text(json.dumps({"reviewed": True}))  # no WAV
    write_wav(tmp_path / "snapshot-x-missed.wav", [0.0])  # no time anywhere
    (tmp_path / "snapshot-x-missed.json").write_text(json.dumps({"reviewed": True}))
    stem = add_clip(tmp_path, day_unix("2026-11-13", 14))
    side = json.loads((tmp_path / f"{stem}.json").read_text())
    side.update(seconds=None, sample_rate=None, unix_time=None)  # nulls: defaults / stem time
    (tmp_path / f"{stem}.json").write_text(json.dumps(side))
    clips, skipped = snap.read_clips(tmp_path, UTC)
    reasons = dict(skipped)
    assert reasons["snapshot-1-missed"].startswith("unreadable sidecar")
    assert reasons["snapshot-2-missed"] == "WAV missing"
    assert reasons["snapshot-x-missed"] == "no time"
    assert [c.stem for c in clips] == [f"snapshot-{day_unix('2026-11-13')}-missed", stem]
    assert clips[1].unix_time == day_unix("2026-11-13", 14), "time from the file name"


def test_invalid_region_bounds_are_dropped() -> None:
    events = snap._events(
        [smack(0.1, 0.2), {"start_s": float("nan"), "end_s": 0.3}, smack(0.5, 0.4), smack(-1, 0.1), {"x": 1}, "junk"],
        "lip_smack",
    )
    assert events == [snap.Event(0.1, 0.2, "lip_smack")]


def test_auto_clips_are_recognised_by_name_too() -> None:
    assert snap.is_auto("snapshot-1-auto", "")
    assert snap.is_auto("snapshot-1-auto-2-imported1", "")
    assert snap.is_auto("snapshot-1-missed", "auto")
    assert not snap.is_auto("snapshot-1-missed", "missed")
    assert not snap.is_auto("snapshot-1-false_hit", "")


def test_a_session_across_midnight_is_one_day(tmp_path: Path) -> None:
    late = day_unix("2026-11-13", 23) + 40 * 60  # 23:40
    add_clip(tmp_path, late, regions=[smack(0.5, 0.52)])
    add_clip(tmp_path, late + 40 * 60, label="false_hit")  # 00:20, same call
    add_clip(tmp_path, late + 10 * 3600)  # next morning: a new session
    clips, _ = snap.read_clips(tmp_path, UTC)
    assert [c.day for c in clips] == ["2026-11-13", "2026-11-13", "2026-11-14"]


def test_split_fallbacks_never_leave_a_set_without_training_data(monkeypatch: pytest.MonkeyPatch) -> None:
    days = [f"2026-11-1{d}" for d in range(6)]
    # Every day hashes into test: a usable split still has train, test and val days.
    monkeypatch.setattr(snap, "_day_hash", lambda d: 0.01 * days.index(d))
    split = snap.assign_splits(days)
    assert {"train", "test", "val"} <= set(split.values())
    assert set(snap.assign_splits(days[:2]).values()) >= {"train"}


def test_wav_reader_rejects_truncated_and_unsupported_files(tmp_path: Path) -> None:
    fmt = struct.pack("<HHIIHH", 1, 2, SR, SR * 6, 6, 24)  # 24-bit PCM: not ours
    body = b"WAVE" + b"fmt " + struct.pack("<I", 16) + fmt + b"data" + struct.pack("<I", 6) + bytes(6)
    (tmp_path / "24.wav").write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
    with pytest.raises(ValueError, match="unsupported"):
        snap.read_wav_mono(tmp_path / "24.wav")
    body = b"WAVE" + b"fmt " + struct.pack("<I", 4) + b"\x03\x00\x02\x00"  # fmt cut short
    (tmp_path / "cut.wav").write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
    with pytest.raises(ValueError, match="truncated"):
        snap.read_wav_mono(tmp_path / "cut.wav")


def test_a_training_clip_with_unreadable_audio_is_left_out(tmp_path: Path) -> None:
    folder, out = tmp_path / "snaps", tmp_path / "out"
    folder.mkdir()
    stem = add_clip(folder, day_unix("2026-11-13"), regions=[smack(0.3, 0.31)])
    (folder / f"{stem}.wav").write_bytes(b"RIFF....WAVEjunk")
    summary = snap.write(folder, out, UTC)
    assert summary["clips"] == 0
    assert summary["skipped_by_reason"] == {"unreadable WAV": 1}
    assert (out / "manifest.jsonl").read_text() == ""


def test_leakage_check_rejects_pool_events_from_other_splits(tmp_path: Path) -> None:
    c = snap.Clip("a", tmp_path / "a.wav", "2026-11-13", 0, "missed", False, 1.0, SR)
    c.split = "test"
    with pytest.raises(ValueError, match="pool"):
        snap.check_no_leakage([c], [{"stem": "a", "day": "2026-11-13"}])


def test_locked_days_keep_their_split_when_new_days_arrive(tmp_path: Path) -> None:
    three = ["2026-11-13", "2026-11-14", "2026-11-15"]
    first = snap.assign_splits(three)
    assert "test" in first.values()
    # A later day that hashes into test doesn't push the first test day into training/val.
    assert snap._day_hash("2026-11-19") < snap.TEST_SHARE
    later = snap.assign_splits([*three, "2026-11-19"], locked=first)
    assert all(later[d] == first[d] for d in three)
    assert later["2026-11-19"] == "test"
    # Without the lock the fallback test day would have moved.
    assert snap.assign_splits([*three, "2026-11-19"]) != later


def test_fallback_test_day_avoids_the_auto_clip_day() -> None:
    days = ["2026-11-13", "2026-11-14", "2026-11-15"]
    autos = {d: 0 for d in days}
    plain = snap.assign_splits(days, auto_clips=autos)
    test_day = next(d for d, s in plain.items() if s == "test")
    autos[test_day] = 180
    moved = snap.assign_splits(days, auto_clips=autos)
    assert moved[test_day] != "test" and "test" in moved.values()


def test_pool_isolation_counts_duplicates_and_rejected_spans() -> None:
    audio = array("f", clip_audio(1.0, ((0.30, 0.31, 0.5), (0.60, 0.61, 0.5))))
    c = snap.Clip("s", Path("s.wav"), "2026-11-13", 0, "missed", False, 1.0, SR)
    c.split = "train"
    c.regions = [snap.Event(0.30, 0.31, "lip_smack"), snap.Event(0.30, 0.31, "lip_smack")]  # marked twice
    assert snap.pool_events(c, audio, SR) == []
    c.regions = [snap.Event(0.60, 0.61, "lip_smack")]
    c.rejected = [snap.Event(0.65, 0.66, "rejected")]  # a hard negative 40 ms later
    assert snap.pool_events(c, audio, SR) == []


def test_regions_outside_the_clip_are_dropped() -> None:
    events = snap._events([smack(0.1, 0.2), smack(7.9, 8.5), smack(0.3, 0.3)], "lip_smack", seconds=8.0)
    assert events == [snap.Event(0.1, 0.2, "lip_smack")]


def test_write_records_meta_lock_and_warnings(tmp_path: Path) -> None:
    folder, out = tmp_path / "snaps", tmp_path / "out"
    folder.mkdir()
    for d in ["2026-11-13", "2026-11-14", "2026-11-15"]:
        add_clip(folder, day_unix(d), regions=[smack(0.3, 0.31)])
    summary = snap.write(folder, out, UTC)
    meta = json.loads((out / "meta.json").read_text())
    lock = json.loads((out / snap.LOCK).read_text())
    assert meta["manifest_sha256"] == summary["manifest_sha256"] == snap._sha256(out / "manifest.jsonl")
    assert meta["splits"] == lock and set(lock) == {"2026-11-13", "2026-11-14", "2026-11-15"}
    assert any("rough" in w for w in summary["warnings"])
    # A second run with one more day keeps the locked days.
    add_clip(folder, day_unix("2026-11-19"), regions=[smack(0.3, 0.31)])
    snap.write(folder, out, UTC)
    again = json.loads((out / snap.LOCK).read_text())
    assert {d: again[d] for d in lock} == lock
    (out / snap.LOCK).write_text("{ broken")
    with pytest.raises(ValueError, match="split lock"):
        snap.write(folder, out, UTC)


def test_main_accepts_a_time_zone(tmp_path: Path) -> None:
    folder = tmp_path / "snaps"
    folder.mkdir()
    add_clip(folder, day_unix("2026-11-13", 23) + 30 * 60)  # 23:30 UTC = 00:30 in Stockholm
    out = tmp_path / "out"
    assert snap.main(["--snapshots", str(folder), "--out", str(out), "--tz", "Europe/Stockholm"]) == 0
    assert json.loads((out / "meta.json").read_text())["splits"] == {"2026-11-14": "train"}
