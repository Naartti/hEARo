# hEARo — agent instructions

hEARo is a macOS 26+ menu bar app that removes mouth clicks, smacks, sibilance, hum and
plosives from system audio in real time. Rust owns the whole audio path; Swift owns only the UI.
The full brief is docs/HANDOVER.md; current state and next task are in docs/PROGRESS.md.

## Start of every session

1. Read docs/PROGRESS.md (current milestone, next task) and `git log --oneline -20`.
2. Read the doc for the area you'll touch (docs/ARCHITECTURE.md, docs/DESIGN.md, docs/adr/).
3. If tools seem off, run `just doctor`.

## Local CI — there is no hosted CI (ADR 0009)

Everything runs on this Mac through `just` → `scripts/*.sh`. Never add GitHub Actions.

| When | Run | What it covers |
|---|---|---|
| Before **every** commit | `just check` (≈5 s warm, < 60 s) | rustfmt, clippy `-D warnings`, all fast Rust tests (incl. property, RT-safety, and a stress test simulating 10 min of audio in < 1 s), Swift build + tests, ruff + pytest, repo hygiene |
| Before every push, at **milestone end**, after any dependency/toolchain bump, after touching `deny.toml`/`justfile`/`scripts/` | `just ci` | `check` + golden tests + doc tests + CPU budgets (release) + cargo-deny + cargo-machete + XcodeGen spec + markdownlint + offline link check. Log: `target/ci/last.log` |
| After touching DSP module code | `just test-golden`, `just bench` | measurable quality thresholds; CPU budget per block |
| Docs-only change | `just docs` | lint + links |

Git hooks (lefthook) enforce this: pre-commit = fmt/ruff/markdownlint/hygiene on staged files,
commit-msg = Conventional Commits, pre-push = `just ci`. Never bypass with `--no-verify`.
A red gate is never committed: fix the cause, don't skip the step.

## Other commands (always via just)

- `just cli process in.wav out.wav --modules <list> --report`  offline DSP iteration
- `just signals`  writes reference signals to scratch/signals
- `just cli procs [--watch]` / `just cli run --modules gain --stats`  live engine from the terminal
- `just ffi`  Rust engine → XCFramework + UniFFI Swift (app/RustBridge, generated; auto-run by
  `just check` and `just app`)
- `just app` / `just run`  build (SwiftPM), sign with stable identity, install
  `~/Applications/hEARo.app` (one app for debug and release, docs/dev/SIGNING.md), run it
  (`run` streams `dev.hearo` logs; the app adds its os_log logger in M3)
- `just xcode`  optional: generate + open the Xcode project

## Map

```text
crates/hearo-dsp     pure DSP modules + chain (no OS deps)
crates/hearo-testkit seeded test signals, metrics, WAV, allocation counter
crates/hearo-ml      streaming model runner (M8)
crates/hearo-engine  Core Audio taps, aggregate device, IOProc, watchdog, telemetry (M2)
crates/hearo-ffi     UniFFI control plane for Swift (M3)
crates/hearo-cli     `hearo` dev CLI
app/                 SwiftPM: HearoKit (all UI, no Rust) + HearoLive (RustEngine, hotkey) + thin @main
ml/                  Python (uv) data, training, eval, export
scripts/             the local CI and build scripts
docs/                doc-as-code; deep-dives/ explain everything
```

## Non-negotiable rules

- Real-time code (anything reachable from `Module::process`, `Chain::process` or the IOProc):
  no allocation, locks, blocking, I/O, logging, panics or unbounded loops. Tests count
  allocations with `hearo_testkit::rt::count_allocs` (ADR 0007) and must see zero.
- Global bypass is bit-exact passthrough. Never leave the user in silence; fail to passthrough.
- Test first. Never weaken or delete a test to get green. Golden updates need a reason in the commit.
- Docs change in the same commit as the code they describe.
- Conventional Commits, one logical change each. Update docs/PROGRESS.md after each task.
- Verify crate/tool versions and APIs before using them; pin exact versions.
- No network code, telemetry, secrets, audio recordings or large binaries in git.
- Secrets live in `.env` / `.env.local` (git-ignored; template `.env.example`). Never print, log
  or commit them; only download scripts read `HUGGINGFACE_API_KEY` from the environment.
- Generated files (app/*.xcodeproj, app/RustBridge/generated/) are never hand-edited.
- Print lints (`clippy::print_stdout`) are workspace-wide; only binaries/tests that exist to print
  may allow them, at the crate/file root with a comment.

## Gotchas

- A tap-backed aggregate's IOProc only runs while some tapped app plays audio: zero callbacks
  = idle, not an error (ADR 0010). Permission is checked via TCC preflight (`ca::permission`).
  Diagnose (with audio playing): `cargo run -p hearo-engine --example tap_diagnose`.
- Ad-hoc signing loses TCC permission on every rebuild; `just app` signs with a stable identity.
- Electron apps (Slack, Teams) play audio from helper processes; prefer global tap minus exclusions.
- Always exclude hEARo's own process from the tap (feedback loop otherwise).
- Rebuild tap AND aggregate device on output/sample-rate changes; restarting the IOProc isn't enough.
- `#Preview` macros need Xcode; use `PreviewProvider` so HearoKit builds with Command Line Tools.
- Swift Testing needs Xcode; without it `scripts/swift-check.sh` builds only and warns.
- Don't run `hearo run` and the app at the same time: each would tap the other's output.
- Test profile uses opt-level 2 so long golden/stress tests stay fast.
