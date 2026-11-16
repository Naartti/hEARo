"""Event scoring: the same cases as the Rust unit tests in crates/hearo-cli/src/eval.rs."""

from __future__ import annotations

import numpy as np
import pytest

from hearo_ml.scoring import Labels, Score, Span, detections, labels_from_clip, score, wilson


def sp(a: float, b: float) -> Span:
    return Span(a, b)


def test_classifies_detections_and_counts_regions():
    labels = Labels(targets=(sp(1.0, 1.03), sp(3.0, 3.02)), neutral=(sp(5.0, 5.4),), rejected=(sp(7.0, 7.02),))
    dets = [sp(1.01, 1.02), sp(5.1, 5.2), sp(7.0, 7.01), sp(8.0, 8.01), sp(1.02, 1.04)]
    s = score(dets, labels)
    assert (s.regions, s.hit_regions, s.detections, s.neutral, s.false_positives, s.rejected_hits) == (2, 1, 5, 1, 2, 1)
    assert s.precision() == pytest.approx(2 / 4)
    assert s.recall() == pytest.approx(0.5)


def test_tolerance_is_applied_both_ways():
    labels = Labels(targets=(sp(2.0, 2.01),))
    assert score([sp(2.025, 2.03)], labels).hit_regions == 1
    assert score([sp(1.97, 1.985)], labels).hit_regions == 1
    assert score([sp(2.04, 2.05)], labels).hit_regions == 0


def test_warmup_drops_early_labels_and_detections():
    labels = labels_from_clip(
        [{"start_s": 0.2, "end_s": 0.22, "cls": "lip_smack"}, {"start_s": 2.0, "end_s": 2.05, "cls": "breath"}],
        [{"start_s": 3.0, "end_s": 3.1, "cls": "rejected"}],
    )
    assert labels.targets == () and labels.neutral == (sp(2.0, 2.05),) and labels.rejected == (sp(3.0, 3.1),)
    s = score([sp(0.1, 0.12)], labels, seconds=8.0)
    assert s.detections == 0 and s.seconds == pytest.approx(7.5)


def test_runs_of_frames_become_merged_spans():
    p = np.array([0, 0.9, 0.9, 0, 0, 0, 0, 0.6, 0, 0.7, 0], dtype=np.float32)
    # Gap of 1 frame (10 ms) merges; a gap of 4 frames doesn't.
    assert detections(p, 0.5, 0.01) == [sp(0.01, 0.03), sp(0.07, 0.10)]
    assert detections(p, 0.95, 0.01) == []


def test_f1_and_fp_rate():
    s = Score(regions=10, hit_regions=8, detections=12, neutral=2, false_positives=2, seconds=120.0)
    assert s.precision() == pytest.approx(0.8)
    assert s.f1() == pytest.approx(0.8)
    assert s.fp_per_min() == pytest.approx(1.0)
    assert Score().f1() is None


def test_wilson_interval():
    lo, hi = wilson(17, 17)
    assert hi == 1.0 and 0.80 < lo < 0.82
    lo, hi = wilson(5, 10)
    assert lo == pytest.approx(0.2366, abs=1e-3) and hi == pytest.approx(0.7634, abs=1e-3)
    assert wilson(0, 0) is None
