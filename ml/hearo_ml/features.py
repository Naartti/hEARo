"""Audio loading and causal log-mel frames for the mouth-noise detector (HANDOVER §7.6).

Frames follow the runtime's STFT (20 ms Hann window, 10 ms hop) at a fixed internal rate, so
`hearo-ml` (M8) can compute the same features in Rust:

- Frame `t` uses the **last** `WIN` samples ending at sample `(t + 1) * HOP` (zero-padded at the
  start of a clip): it never looks at audio after its own hop, so it's causal.
- Its *own* time is that newest hop, `[t * HOP, (t + 1) * HOP)` in samples: labels and
  detections refer to it (`frame_span`).
- `N_MELS` triangular mel bands (Slaney-style, area-normalised) between `FMIN` and `FMAX`, power
  in dB with a floor (`LOG_FLOOR_DB`).

32 kHz keeps the band up to 16 kHz where smacks are bright, and above what 16 to 32 kbit/s Opus
keeps anyway.
"""

from __future__ import annotations

from fractions import Fraction
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

SR = 32_000
WIN = 640  # 20 ms
HOP = 320  # 10 ms
N_FFT = 1024
N_MELS = 48
FMIN = 50.0
FMAX = 16_000.0
LOG_FLOOR_DB = -100.0
HOP_S = HOP / SR


def resample(x: np.ndarray, rate: float, to: int = SR) -> np.ndarray:
    """Polyphase resampling of a mono signal from `rate` to `to` Hz."""
    if int(rate) == to:
        return x.astype(np.float32, copy=False)
    f = Fraction(to, round(rate))
    return resample_poly(x, f.numerator, f.denominator).astype(np.float32)


def load_audio(path: Path) -> np.ndarray:
    """A WAV (or anything libsndfile reads) as mono float32 at `SR`."""
    x, rate = sf.read(str(path), dtype="float32", always_2d=True)
    return resample(x.mean(axis=1), rate)


def _hz_to_mel(f: np.ndarray) -> np.ndarray:
    return 2595.0 * np.log10(1.0 + f / 700.0)


def _mel_to_hz(m: np.ndarray) -> np.ndarray:
    return 700.0 * (10.0 ** (m / 2595.0) - 1.0)


def mel_filterbank() -> np.ndarray:
    """`[N_MELS, N_FFT // 2 + 1]` triangular filters, each with unit area (in bins)."""
    edges = _mel_to_hz(np.linspace(_hz_to_mel(np.array(FMIN)), _hz_to_mel(np.array(FMAX)), N_MELS + 2))
    bins = np.fft.rfftfreq(N_FFT, 1.0 / SR)
    fb = np.zeros((N_MELS, bins.size), dtype=np.float64)
    for m in range(N_MELS):
        lo, mid, hi = edges[m], edges[m + 1], edges[m + 2]
        up = (bins - lo) / (mid - lo)
        down = (hi - bins) / (hi - mid)
        fb[m] = np.maximum(0.0, np.minimum(up, down))
        area = fb[m].sum()
        if area > 0:
            fb[m] /= area
    return fb.astype(np.float32)


_WINDOW = np.hanning(WIN + 1)[:WIN].astype(np.float32)  # periodic Hann
_FB = mel_filterbank()


def n_frames(n_samples: int) -> int:
    return n_samples // HOP


def log_mel(x: np.ndarray) -> np.ndarray:
    """`[T, N_MELS]` causal log-mel frames of a mono `SR` signal, `T = len(x) // HOP`."""
    t = n_frames(len(x))
    if t == 0:
        return np.zeros((0, N_MELS), dtype=np.float32)
    padded = np.concatenate([np.zeros(WIN - HOP, dtype=np.float32), x[: t * HOP].astype(np.float32)])
    idx = np.arange(WIN)[None, :] + HOP * np.arange(t)[:, None]
    frames = padded[idx] * _WINDOW
    spec = np.fft.rfft(frames, n=N_FFT, axis=1)
    power = (spec.real**2 + spec.imag**2) / (WIN / 2) ** 2  # a full-scale sine ≈ 0 dB
    mel = power @ _FB.T
    return np.maximum(10.0 * np.log10(mel + 1e-12), LOG_FLOOR_DB).astype(np.float32)


def frame_span(t: int) -> tuple[float, float]:
    """The time (s) frame `t` stands for: its newest hop."""
    return t * HOP_S, (t + 1) * HOP_S
