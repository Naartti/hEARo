# Architecture Decision Records

Template: [0000-template.md](0000-template.md). How to write one: `.claude/skills/write-adr/`.

| # | Decision | Status |
|---|---|---|
| [0001](0001-rust-owns-the-audio-path.md) | Rust owns the entire audio path | accepted |
| [0002](0002-uniffi-control-plane.md) | UniFFI for the control plane only | accepted |
| [0003](0003-global-tap-minus-exclusions.md) | Global tap minus exclusions by default | accepted |
| [0004](0004-swiftpm-first-xcodegen-optional.md) | SwiftPM-first app build; XcodeGen optional | accepted |
| [0005](0005-stable-dev-signing.md) | Stable signing identity for dev builds | accepted |
| [0006](0006-ml-runtime-tract-vs-ort.md) | tract vs ort | proposed (M8) |
| [0007](0007-counting-allocator-for-rt-tests.md) | Counting allocator for RT tests | accepted |
| [0008](0008-hardened-runtime-no-sandbox.md) | Hardened Runtime, no sandbox, no network | accepted |
| [0009](0009-local-ci-no-hosted-ci.md) | Local CI only — no hosted CI | accepted |
| [0010](0010-engine-on-objc2-core-audio.md) | Live engine built directly on objc2-core-audio | accepted |
| [0011](0011-desmack-lookahead-and-io-buffer.md) | De-smack 40 ms lookahead, 128-frame IO buffer | accepted |
| [0012](0012-chain-latency-budget.md) | Chain latency budget (sum of lookaheads) | accepted |
| [0013](0013-uniffi-mpl-license-exception.md) | License exception: UniFFI (MPL-2.0) | accepted |
| [0014](0014-engine-supervisor-watchdog-and-wake.md) | Engine supervisor: watchdog and sleep/wake recovery in Rust | accepted |
| [0015](0015-module-settings-and-reduction-columns.md) | Module settings owned by Rust; per-module GR columns | accepted |
