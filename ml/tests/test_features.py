"""Causal log-mel frames, labels per frame, mixing and codec alignment."""

from __future__ import annotations

import shutil

import numpy as np
import pytest

from hearo_ml import dataset as ds
from hearo_ml import features as feat


def tone(freq: float, secs: float, amp: float = 0.5) -> np.ndarray:
    t = np.arange(int(secs * feat.SR)) / feat.SR
    return (amp * np.sin(2 * np.pi * freq * t)).astype(np.float32)


def test_frame_count_and_tone_band():
    x = tone(1000.0, 1.0)
    m = feat.log_mel(x)
    assert m.shape == (100, feat.N_MELS)
    centres = np.argmax(feat.mel_filterbank(), axis=1) * feat.SR / feat.N_FFT
    assert abs(centres[int(np.argmax(m[50]))] - 1000.0) < 150
    # A -6 dBFS sine, averaged over its band's bins (area-normalised filters): well above the
    # floor, below full scale.
    assert -30.0 < m[50].max() < 0.0


def test_frames_are_causal():
    x = tone(500.0, 1.0)
    y = x.copy()
    y[5000:] = np.random.default_rng(0).standard_normal(len(y) - 5000).astype(np.float32)
    a, b = feat.log_mel(x), feat.log_mel(y)
    last_clean = 5000 // feat.HOP - 1  # frame t ends at sample (t + 1) * HOP
    assert np.array_equal(a[: last_clean + 1], b[: last_clean + 1])
    assert not np.array_equal(a[last_clean + 1], b[last_clean + 1])


def test_resample_and_silence_floor():
    x = feat.resample(np.zeros(44_100, dtype=np.float32), 44_100)
    assert len(x) == feat.SR
    assert np.all(feat.log_mel(x) == feat.LOG_FLOOR_DB)


def test_frame_targets_mark_events_and_skip_neutral_and_edges():
    regions = [
        {"start_s": 0.105, "end_s": 0.125, "cls": "lip_smack"},
        {"start_s": 0.5, "end_s": 0.55, "cls": "breath"},
    ]
    rejected = [{"start_s": 0.8, "end_s": 0.82, "cls": "rejected"}]
    y, w = ds.frame_targets(100, regions, rejected, hard_negative_weight=3.0)
    assert list(np.flatnonzero(y)) == [10, 11, 12]
    assert w[9] == 0 and w[13] == 0 and w[10] == 1  # hand-drawn edges aren't scored
    assert np.all(w[50:55] == 0) and np.all(y[50:55] == 0)
    assert np.all(w[80:82] == 3.0)
    assert w[30] == 1 and y[30] == 0


def test_other_detectors_rejections_are_not_hard_negatives():
    rejected = [
        {"start_s": 0.3, "end_s": 0.32, "cls": "lip_smack", "module": "desmack"},
        {"start_s": 0.6, "end_s": 0.62, "cls": "sibilance", "module": "deess"},
    ]
    _, w = ds.frame_targets(100, [], rejected, hard_negative_weight=3.0)
    assert np.all(w[30:32] == 3.0) and np.all(w[60:62] == 1.0)


def test_cache_key_follows_codecs_and_synthetic_availability():
    cfg = ds.DataConfig()
    keys = {ds.cache_key("a", cfg, c, s) for c in (True, False) for s in (True, False)}
    assert len(keys) == 4
    assert ds.cache_key("a", cfg) != ds.cache_key("b", cfg)


def test_mix_adds_labelled_events_at_the_requested_level():
    rng = np.random.default_rng(1)
    bg = 0.05 * np.random.default_rng(2).standard_normal(8 * feat.SR).astype(np.float32)
    ev = ds.cut_event(np.ones(feat.SR, dtype=np.float32), 0.1, 0.12)
    cfg = ds.DataConfig(events_per_mix=(3, 3), esr_db=(0.0, 0.0))
    out, regs = ds.mix(bg, [{"start_s": 4.0, "end_s": 4.02, "cls": "lip_smack"}], [ev], cfg, rng)
    assert len(regs) == 4
    for r in regs:
        if r["start_s"] == 4.0:
            continue
        a, b = int(r["start_s"] * feat.SR), int(r["end_s"] * feat.SR)
        added = out[a:b] - bg[a:b]
        assert abs(ds._rms_db(added) - ds.active_rms_db(bg)) < 2.0
        assert not any(abs(r["start_s"] - o["start_s"]) < 0.05 for o in regs if o is not r)


def test_align_undoes_a_delay():
    x = np.random.default_rng(3).standard_normal(feat.SR).astype(np.float32)
    y = np.concatenate([np.zeros(312, dtype=np.float32), x])[: len(x)]
    assert np.allclose(ds.align(x, y)[: len(x) - 400], x[: len(x) - 400])


@pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")
@pytest.mark.parametrize(("codec", "kbps"), [("opus", 24), ("aac", 96)])
def test_codec_roundtrip_keeps_events_in_place(codec, kbps):
    x = 0.01 * np.random.default_rng(4).standard_normal(2 * feat.SR).astype(np.float32)
    x[feat.SR : feat.SR + 200] += 0.8
    y = ds.codec_roundtrip(x, codec, kbps, shutil.which("ffmpeg"))
    assert len(y) == len(x)
    assert abs(int(np.argmax(np.abs(y))) - feat.SR) < feat.HOP  # within one frame
