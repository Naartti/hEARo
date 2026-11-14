# 0012. Chain latency budget

Date: 2026-11-13 · Status: accepted

## Context

Module lookaheads add up: the chain delays the signal by the **sum** of all modules'
`latency_samples()` (each slot keeps a dry delay of its own module's lookahead, ADR in
ARCHITECTURE "The chain"). HANDOVER §2: added latency target ≤ 30 ms, hard ceiling 45 ms.
De-smack needs 40 ms of lookahead to tell lip smacks from stop consonants (ADR 0011), and the
engine's IO buffer adds ~3 ms (128 frames).

## Decision

Lookahead budget per module (at any sample rate):

| Module | Lookahead | Notes |
|---|---|---|
| De-hum | 0 | IIR notches |
| De-plosive | 0 | fast detector, dynamic low cut |
| De-click | ≤ 2.5 ms | AR interpolation needs a little future context; longer clicks are de-smack's job |
| De-smack | 40 ms | ADR 0011 |
| De-ess | 0 | fast attack instead of lookahead |
| Limiter | ≤ 0.5 ms | true-peak with short lookahead + sliding max |
| **Total** | **≤ 43 ms** + IO buffer ≈ **46 ms worst case** | |

The total slightly exceeds the 45 ms ceiling only if every module is enabled at the maximum
budgets; measured values are reported live in the status line. A future "low latency" preset
(HANDOVER §17) disables de-smack for calls (≈ 6 ms total).

## Alternatives considered

- **Shared lookahead** (all detectors look at the same future window, all gains applied to one
  delayed signal → latency = max, not sum). The right long-term architecture; needs modules
  split into detector + applier halves. Revisit if another module needs real lookahead.
- **Raise the ceiling.** Rejected without the human's say-so.

**Update (overnight, all six modules built):** measured live 45.9 ms with a 128-frame IO buffer
(0.9 ms over the ceiling) → the engine now asks for a **64-frame buffer** (1.45 ms at 44.1 kHz):
measured **≈ 44.5 ms** added, 48–64 µs per 1.45 ms cycle, 0 overruns over 8 s with every module
on. Shared lookahead remains the proper fix (it would bring the total to ≈ 42 ms).

## Consequences (incl. how we'd know it was wrong)

- New modules must justify any lookahead against this table (dsp-reviewer checks it).
- Wrong if: the human notices lip-sync or call delay, or a module can't meet its spec within
  its budget — then implement shared lookahead.
