# 0011. De-smack: 40 ms lookahead, 128-frame IO buffer

Date: 2026-11-13 · Status: accepted

## Context

The pause-aware de-smack must not duck **stop consonants** ("t", "k", "p", "b", "d", "g"):
a closure looks like a pause and the burst decays as fast as a smack. What separates them is
what follows: a smack in a pause is followed by quiet, a stop by aspiration or voicing within
5–20 ms. Waiting to see that quiet before the burst reaches the output costs lookahead.

Measured on synthetic stops after closures (172 bursts) with smack recall unchanged (93–95 %):

| Lookahead / required quiet | Stops ducked |
|---|---|
| 30 ms / 10 ms | 26 % |
| 35 ms / 15 ms | 15 % |
| **40 ms / 20 ms** | **2–4 %** |

HANDOVER §2: added latency target ≤ 30 ms, hard ceiling 45 ms. With the default 512-frame IO
buffer (11.6 ms at 44.1 kHz), 40 ms lookahead would be ~52 ms.

## Decision

De-smack uses a 40 ms lookahead and requires 20 ms of quiet after the burst. The engine asks
for a **128-frame IO buffer** on its aggregate device (2.9 ms at 44.1 kHz, best effort), so the
added latency is ≈ 43 ms — over the 30 ms target, under the 45 ms ceiling. Processing cost
per 128-frame cycle is ~25–35 µs of a 2.9 ms budget; 0 overruns observed.

## Alternatives considered

- **30 ms / 10 ms** (≈ 33 ms latency): a quarter of stop consonants dulled — worse than an
  occasional missed smack.
- **Spectral discrimination** (smack vs stop): synthetic models overlap by construction; not
  trustworthy without real data. Revisit with the ML stage (labeled snapshots).
- **Provisional duck + cancel**: the damage (a ducked consonant) is already audible.

## Consequences (incl. how we'd know it was wrong)

- Audio is ~43 ms late: below the commonly cited lip-sync detectability for late audio
  (≈ 100+ ms), noticeable only as slightly more call latency. A "low latency" preset that turns
  de-smack off (HANDOVER §17) remains the escape hatch for calls.
- Wrong if: the human hears consonants dulled (use `f` "false hit" snapshots), notices lip-sync
  issues, or Bluetooth devices refuse 128-frame buffers (latency then ≈ 52 ms — measure).
