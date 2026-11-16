"""Event-level scoring, the same rules as `hearo dataset eval` (crates/hearo-cli/src/eval.rs).

A reviewed clip is exhaustively labelled (ADR 0017), so:

- a detection overlapping a **target** region (lip smack, mouth click; ±`TOLERANCE_S`) is a hit;
- one on a **neutral** region (breath, filled pause, …) is neither right nor wrong;
- anything else is a **false positive**; those on a human-**rejected** span are counted too;
- detections and labels starting in the first `WARMUP_S` are dropped (the DSP detector is
  cold-started there; the model is held to the same window so the numbers compare).

The DSP baseline is re-scored here from `hearo dataset eval --json`, so both sides run through
this one function. `tests/test_scoring.py` mirrors the Rust unit tests.
"""

from __future__ import annotations

import math
from collections.abc import Iterable, Sequence
from dataclasses import dataclass

import numpy as np

TARGET_CLASSES = ("lip_smack", "mouth_click")
TOLERANCE_S = 0.02
WARMUP_S = 0.5
#: Detections closer than this merge into one (a smack split over two runs is one event).
MERGE_GAP_S = 0.03


@dataclass(frozen=True)
class Span:
    start: float
    end: float

    def overlaps(self, o: Span, tol: float = TOLERANCE_S) -> bool:
        return self.start <= o.end + tol and o.start <= self.end + tol


@dataclass(frozen=True)
class Labels:
    targets: tuple[Span, ...] = ()
    neutral: tuple[Span, ...] = ()
    rejected: tuple[Span, ...] = ()


@dataclass
class Score:
    regions: int = 0
    hit_regions: int = 0
    detections: int = 0
    neutral: int = 0
    false_positives: int = 0
    rejected_hits: int = 0
    #: Scored audio (after the warm-up), for false positives per minute.
    seconds: float = 0.0

    def add(self, o: Score) -> None:
        for k in vars(self):
            setattr(self, k, getattr(self, k) + getattr(o, k))

    @property
    def hits(self) -> int:
        """Detections on a target (precision's numerator)."""
        return self.detections - self.neutral - self.false_positives

    def recall(self) -> float | None:
        return self.hit_regions / self.regions if self.regions else None

    def precision(self) -> float | None:
        judged = self.detections - self.neutral
        return self.hits / judged if judged else None

    def f1(self) -> float | None:
        p, r = self.precision(), self.recall()
        if p is None or r is None or p + r == 0:
            return None if p is None or r is None else 0.0
        return 2 * p * r / (p + r)

    def fp_per_min(self) -> float | None:
        return 60.0 * self.false_positives / self.seconds if self.seconds > 0 else None


def labels_from_clip(regions: Iterable[dict], rejected: Iterable[dict]) -> Labels:
    """Labels from a manifest line's `regions` / `rejected`, minus those in the warm-up."""

    def keep(items: Iterable[dict], pred) -> tuple[Span, ...]:
        return tuple(Span(r["start_s"], r["end_s"]) for r in items if pred(r) and r["start_s"] >= WARMUP_S)

    regions, rejected = list(regions), list(rejected)
    return Labels(
        targets=keep(regions, lambda r: r["cls"] in TARGET_CLASSES),
        neutral=keep(regions, lambda r: r["cls"] not in TARGET_CLASSES),
        rejected=keep(rejected, lambda r: True),
    )


def score(detections: Sequence[Span], labels: Labels, seconds: float = 0.0) -> Score:
    dets = [d for d in detections if d.start >= WARMUP_S]
    s = Score(regions=len(labels.targets), detections=len(dets), seconds=max(0.0, seconds - WARMUP_S))
    for d in dets:
        if any(d.overlaps(t) for t in labels.targets):
            continue
        if any(d.overlaps(t) for t in labels.neutral):
            s.neutral += 1
        else:
            s.false_positives += 1
            s.rejected_hits += any(d.overlaps(t) for t in labels.rejected)
    s.hit_regions = sum(any(d.overlaps(t) for d in dets) for t in labels.targets)
    return s


def detections(probs: np.ndarray, threshold: float, hop_s: float, merge_gap_s: float = MERGE_GAP_S) -> list[Span]:
    """Runs of frames at or above `threshold` → spans (frame `t` = `[t, t + 1) * hop_s`)."""
    on = np.asarray(probs) >= threshold
    out: list[Span] = []
    t, n = 0, len(on)
    while t < n:
        if not on[t]:
            t += 1
            continue
        start = t
        while t < n and on[t]:
            t += 1
        span = Span(start * hop_s, t * hop_s)
        if out and span.start - out[-1].end < merge_gap_s:
            out[-1] = Span(out[-1].start, span.end)
        else:
            out.append(span)
    return out


def wilson(k: int, n: int, z: float = 1.96) -> tuple[float, float] | None:
    """95 % Wilson score interval for `k` successes out of `n`."""
    if n <= 0:
        return None
    p = k / n
    denom = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / denom
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denom
    return max(0.0, centre - half), min(1.0, centre + half)
