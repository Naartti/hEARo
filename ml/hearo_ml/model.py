"""Phase A mouth-noise detector (HANDOVER §7.6): log-mel frames → P(mouth noise) per frame.

`Detector` is a small streaming model: two causal convolutions (kernel 3, the second dilated)
then a GRU and a linear head. Causal = output `t` depends only on frames `≤ t`; `step` runs it
one frame at a time with carried state (the conv history and the GRU state), which is what the
Rust runner does (M8). A test checks `step` against `forward` within 1e-4.

**Lookahead** (`lookahead` frames, 10 ms each): output `t` is trained to say whether frame
`t - lookahead` is a mouth noise, so the model sees that much audio *after* the event before
deciding — the same trade de-smack makes with its 40 ms (ADR 0011). `predict` realigns the
outputs to the frames they describe.

`bidirectional=True` builds the **teacher**: symmetric convolutions and a bidirectional GRU that
see the whole clip. It can't stream; it's the offline upper bound and, later, the checker that
ranks auto-saved clips (it never deletes them — rejected catches are the hard negatives).
"""

from __future__ import annotations

from dataclasses import asdict, dataclass

import numpy as np
import torch
from torch import nn


@dataclass(frozen=True)
class DetectorConfig:
    n_mels: int = 48
    channels: int = 64
    hidden: int = 64
    lookahead: int = 3
    bidirectional: bool = False
    #: Dropout before the GRU, training only (no effect on `predict` / `step`).
    dropout: float = 0.0


class Detector(nn.Module):
    def __init__(self, cfg: DetectorConfig) -> None:
        super().__init__()
        self.cfg = cfg
        c, h = cfg.channels, cfg.hidden
        self.register_buffer("mean", torch.zeros(cfg.n_mels))
        self.register_buffer("std", torch.ones(cfg.n_mels))
        self.conv1 = nn.Conv1d(cfg.n_mels, c, kernel_size=3)
        self.conv2 = nn.Conv1d(c, c, kernel_size=3, dilation=2)
        self.drop = nn.Dropout(cfg.dropout)
        self.gru = nn.GRU(c, h, batch_first=True, bidirectional=cfg.bidirectional)
        self.head = nn.Linear(h * (2 if cfg.bidirectional else 1), 1)

    # Receptive history of the two convolutions: 2 frames + 4 frames.
    HIST1, HIST2 = 2, 4

    def set_normalisation(self, mean: np.ndarray, std: np.ndarray) -> None:
        self.mean.copy_(torch.as_tensor(mean, dtype=torch.float32))
        self.std.copy_(torch.as_tensor(np.maximum(std, 1e-3), dtype=torch.float32))

    def _pad(self, x: torch.Tensor, n: int) -> torch.Tensor:
        # x: [B, C, T]. Causal: all padding before; teacher: split evenly.
        if self.cfg.bidirectional:
            return nn.functional.pad(x, (n // 2, n - n // 2))
        return nn.functional.pad(x, (n, 0))

    def forward(self, feats: torch.Tensor) -> torch.Tensor:
        """`[B, T, n_mels]` → logits `[B, T]` (output `t` describes frame `t - lookahead`)."""
        x = ((feats - self.mean) / self.std).transpose(1, 2)
        x = torch.relu(self.conv1(self._pad(x, self.HIST1)))
        x = torch.relu(self.conv2(self._pad(x, self.HIST2)))
        y, _ = self.gru(self.drop(x).transpose(1, 2))
        return self.head(y).squeeze(-1)

    @property
    def shift(self) -> int:
        """Frames between an input frame and the output that describes it."""
        return 0 if self.cfg.bidirectional else self.cfg.lookahead

    # ── streaming ───────────────────────────────────────────────────────────────────────────

    def initial_state(self) -> dict[str, torch.Tensor]:
        c = self.cfg
        return {
            "x1": torch.zeros(1, c.n_mels, self.HIST1),
            "x2": torch.zeros(1, c.channels, self.HIST2),
            "h": torch.zeros(1, 1, c.hidden),
        }

    def step(self, frame: torch.Tensor, state: dict[str, torch.Tensor]) -> tuple[torch.Tensor, dict]:
        """One `[n_mels]` frame → (logit, new state). Causal models only."""
        assert not self.cfg.bidirectional, "the teacher sees the whole clip; it can't stream"
        x = ((frame - self.mean) / self.std).view(1, -1, 1)
        w1 = torch.cat([state["x1"], x], dim=2)
        a = torch.relu(self.conv1(w1))  # [1, C, 1]
        w2 = torch.cat([state["x2"], a], dim=2)
        b = torch.relu(self.conv2(w2))
        y, h = self.gru(b.transpose(1, 2), state["h"])
        return self.head(y).view(()), {"x1": w1[:, :, 1:], "x2": w2[:, :, 1:], "h": h}

    # ── offline ─────────────────────────────────────────────────────────────────────────────

    @torch.no_grad()
    def predict(self, feats: np.ndarray) -> np.ndarray:
        """P(mouth noise) for each input frame of one clip (`[T, n_mels]` → `[T]`)."""
        if len(feats) == 0:
            return np.zeros(0, dtype=np.float32)
        dev = self.mean.device
        x = torch.as_tensor(feats, dtype=torch.float32, device=dev)
        if self.shift:
            # The last frames' verdicts need `shift` more frames: feed quiet (the floor).
            x = torch.cat([x, x.new_full((self.shift, x.shape[1]), float(feats.min()))])
        p = torch.sigmoid(self.forward(x[None]))[0]
        return p[self.shift :].cpu().numpy().astype(np.float32)

    def n_params(self) -> int:
        return sum(p.numel() for p in self.parameters())

    def to_checkpoint(self) -> dict:
        return {"config": asdict(self.cfg), "state": self.state_dict()}

    @classmethod
    def from_checkpoint(cls, ck: dict) -> Detector:
        m = cls(DetectorConfig(**ck["config"]))
        m.load_state_dict(ck["state"])
        return m.eval()
