# Testing strategy

Full plan: [HANDOVER §10](../HANDOVER.md). What exists now:

| Layer | Where | Status |
|---|---|---|
| Unit tests | `#[cfg(test)]` in every crate | ✔ |
| Property tests (proptest) | `crates/hearo-dsp/tests/chain_properties.rs` | ✔ bypass/all-off bit-exact (from start and mid-stream), block-size invariance (incl. parameter changes), finite output, warm-up alignment, fade length, telemetry timestamps |
| RT safety | `crates/hearo-dsp/tests/rt_safety.rs` | ✔ zero allocations (all toggle states, fault path, event overflow, bad indices), 10-min randomized stress, discontinuity bound, channel identity |
| CPU budgets | `crates/hearo-dsp/tests/budgets.rs` (`just bench`, `just ci`) | ✔ chain ≤ 5% of real time |
| Benchmarks | `crates/hearo-dsp/benches/chain.rs` (divan) | ✔ |
| CLI | `crates/hearo-cli/src/*.rs` unit tests (`render` latency compensation, param parsing, clap) + `tests/cli.rs` end-to-end (bypass bit-exact mono/stereo, report JSON values, error paths) | ✔ |
| Golden audio tests | `crates/hearo-dsp/tests/golden_*.rs` | M4 |
| Engine with FakeBackend | `crates/hearo-engine/tests/` | M2 |
| Swift view models | `app/hEARoTests` (Swift Testing; needs Xcode) | ✔ 6 pass |
| Snapshot tests | `app/hEARoTests` | M3 |
| ML tests | `ml/tests` | smoke only, M7 |
| Hardware | [HARDWARE_CHECKLIST.md](HARDWARE_CHECKLIST.md) | M2 |

Test signals are generated deterministically by `hearo-testkit` (SplitMix64; the stream is pinned
by a test so golden numbers never move because a dependency changed).
