# hEARo — agent instructions

hEARo is a macOS 26+ menu bar app that removes mouth clicks, smacks, sibilance, hum and
plosives from system audio in real time. Rust owns the whole audio path; Swift owns only the UI.
The full brief is docs/HANDOVER.md; current state and next task are in docs/PROGRESS.md.

## Start of every session

1. Read docs/PROGRESS.md (current milestone, next task) and `git log --oneline -20`.
2. Read the doc for the area you'll touch (docs/ARCHITECTURE.md, docs/DESIGN.md, docs/adr/).
3. If tools seem off, run `just doctor`.

## CI — local gate first, GitHub Actions re-runs it (ADR 0016)

Everything runs on this Mac through `just` → `scripts/*.sh`. GitHub Actions
(`.github/workflows/ci.yml`) runs the same `just ci` on every PR and push to `main`, plus
coverage; a `v*` tag runs `release.yml`. Never put logic in a workflow that `just` can't run
locally; pin actions by commit SHA (the hygiene check enforces it).

| When | Run | What it covers |
|---|---|---|
| Before **every** commit | `just check` (≈5 s warm, < 60 s) | rustfmt, clippy `-D warnings`, all fast Rust tests (incl. property, RT-safety, and a stress test simulating 10 min of audio in < 1 s), Swift build + tests, ruff + pytest, repo hygiene |
| Before every push, at **milestone end**, after any dependency/toolchain bump, after touching `deny.toml`/`justfile`/`scripts/` | `just ci` | `check` + golden tests + doc tests + CPU budgets (release) + cargo-deny + cargo-machete + XcodeGen spec + actionlint + markdownlint + offline link check. Log: `target/ci/last.log` |
| After touching DSP module code | `just test-golden`, `just bench` | measurable quality thresholds; CPU budget per block (local only: hosted runners skip budgets) |
| Before pushing a branch that adds Rust code | `just patch-coverage` | Codecov's `codecov/patch` check run locally: coverage of the added lines, uncovered line numbers per file; fails below project coverage − 5 % (`just coverage` alone → `target/coverage/html/index.html`) |
| Docs-only change | `just docs` | lint + links |

Git hooks (lefthook) enforce this: pre-commit = fmt/ruff/markdownlint/hygiene on staged files,
commit-msg = Conventional Commits, pre-push = `just ci`. Never bypass with `--no-verify`.
A red gate is never committed: fix the cause, don't skip the step.

## Any change — feature, fix or refactor

This is the default workflow for every request; the human never has to restate it.

1. **Understand.** Read the docs for the area (docs/DESIGN.md for UI, docs/ARCHITECTURE.md,
   docs/adr/). Add the task to docs/PROGRESS.md (current milestone, or the Backlog section if it
   is off-milestone) with acceptance criteria written from the human's words.
2. **Capture intent, not just the ask.** A request often implies a product or design preference
   ("native", "keep it open while testing", "one window"). Record it in docs/DESIGN.md as a
   dated *human request* note, generalise it into **Principles** when it applies beyond this
   feature, and bring existing UI that now contradicts it into line (or list the leftovers in
   PROGRESS.md as follow-ups).
3. **Decide, don't stall.** Pick the option that fits DESIGN.md, the macOS HIG and existing
   patterns. Record non-trivial choices (ADR via the write-adr skill, or DESIGN.md). Stop only
   for choices that are the human's and costly to undo; put those in docs/OPEN_QUESTIONS.md and
   keep working on everything else.
4. **Branch.** `git switch -c <type>/<slug>` from `main` (type = Conventional Commits type).
5. **Test first, commit as you go.** Failing test → code → `just check` → commit. One logical
   change per commit, every commit green, docs in the same commit, body says *why*.
   Every new code path gets a test, CLI and tooling too: keep I/O at a thin edge (take a
   directory or a writer as an argument, so a test can pass a temp dir or a buffer) and test
   the logic end to end. Before pushing, `just patch-coverage` must pass — a red
   `codecov/patch` on a PR means untested code, not a flaky check (PR #8: 59 %, an untested
   `hearo dataset eval` driver).
6. **Review independently** before calling it done: the area reviewers for what you touched plus
   a general review of the whole branch diff (`main...HEAD`). Fix findings test-first, one
   commit per fix, or note in PROGRESS.md why one was rejected.
7. **Verify for real.** `just ci` green. UI changes: `just app` builds and launches; check what
   you can, then write a short *How to verify* checklist for the human.
8. **Finish.** Tick PROGRESS.md (in the branch), `git push -u origin <branch>` (pre-push runs
   `just ci`), open a PR with `gh pr create` filled from `.github/PULL_REQUEST_TEMPLATE.md`
   (body ends with the How to verify checklist), and wait for `gh pr checks --watch` to go
   green; a red hosted run is fixed like a red local one. The human merges (rebase merge on
   GitHub, branch auto-deleted) unless they asked you to; afterwards `git switch main && git
   pull --ff-only`. Report: the PR link, what changed, decisions made, review findings, and the
   How to verify checklist.

## Dependency updates (Dependabot)

Weekly, and at once for a security alert, run the `dependency-updates` skill (ADR 0019). It is
the one exception to "the human merges": the agent may merge (a) a grouped or security
Dependabot PR whose every bump is semver-compatible and that only touches manifests, lockfiles
or workflow pins, once hosted checks are green and, for Cargo, a local `just ci` (golden + CPU
budgets) passes; (b) a revert of such a merge (revert + `ignore:` entry only). Majors, 0.x
minor bumps, red runs and anything needing code changes follow "Any change"; deferred updates
are ignored and noted under *Dependency notes* in PROGRESS.md.

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
