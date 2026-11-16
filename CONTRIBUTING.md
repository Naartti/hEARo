# Contributing to hEARo

Thanks for your interest! hEARo is a small, pre-1.0 project, so please **open an issue before
starting on anything bigger than a bug fix** — that way we can agree on the approach first.
Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Security
problems go through [SECURITY.md](SECURITY.md), not public issues.

## Getting set up

You need an Apple silicon Mac on macOS 26+ with Homebrew and Xcode (or the Command Line Tools;
Swift tests need Xcode). Then:

```sh
brew install mise
scripts/bootstrap.sh   # pinned toolchains (.mise.toml), Brewfile, Python env, git hooks
just doctor            # checks everything and prints fixes
```

Details: [docs/dev/SETUP.md](docs/dev/SETUP.md). To run the app you also need a stable
signing identity: [docs/dev/SIGNING.md](docs/dev/SIGNING.md). Run `just` to list every
command; use the `just` recipes rather than raw `cargo`/`swift`/`uv` calls.

## CI — run it locally first

The gates are `just` recipes that run on your Mac; GitHub Actions runs the same `just ci` on
every pull request ([ADR 0016](docs/adr/0016-github-actions-ci-and-releases.md)). Run them
locally before you push — it's faster, and hardware tests and CPU budgets only run on a real
Mac. Workflows only call `just`/`scripts/`, and actions are pinned to a commit SHA.

| When | Run |
|---|---|
| Before every commit | `just check` — format, clippy `-D warnings`, all fast Rust tests (unit, property, real-time safety, stress), Swift build + tests, ruff + pytest, repo hygiene |
| Before opening a PR / pushing, and after dependency, toolchain, `justfile` or `scripts/` changes | `just ci` — `check` plus golden audio tests, doc tests, CPU budgets, cargo-deny, cargo-machete, XcodeGen spec, markdownlint and an offline link check |
| After changing DSP module code | `just test-golden` and `just bench` |
| Docs-only changes | `just docs` |

[lefthook](https://lefthook.dev) git hooks (installed by bootstrap) enforce this: pre-commit
runs formatters, markdownlint and the hygiene check on staged files, commit-msg checks the
commit message, and pre-push runs `just ci`. **Never bypass them with `--no-verify`** — a red
gate is fixed, not skipped. The PR's **just ci** check must be green before merging; PRs are
rebase-merged. More: [docs/dev/LOCAL_CI.md](docs/dev/LOCAL_CI.md).

## Rules of the road

- **Conventional Commits**, one logical change per commit: `feat(dsp): …`, `fix(engine): …`,
  `docs: …`. Scopes are lower-case (`[a-z0-9._-]`), one per commit.
- **Test first.** Write the failing test, then the fix. Never weaken or delete a test to get
  green.
- **Golden tests** (`just test-golden`) measure audio quality. Updating a golden snapshot
  (`just golden-update`) needs a reason in the commit message explaining why the numbers
  changed.
- **Real-time safety.** Code reachable from `Module::process`, `Chain::process` or the Core
  Audio IOProc must not allocate, lock, block, do I/O, log, panic or loop unboundedly. Tests
  count allocations with `hearo_testkit::rt::count_allocs` and must see zero
  ([ADR 0007](docs/adr/0007-counting-allocator-for-rt-tests.md),
  [deep dive 02](docs/deep-dives/02-real-time-audio-in-rust.md)).
- **Fail to passthrough.** Global bypass is bit-exact; on any fault the user hears the
  unprocessed audio, never silence.
- **Docs change in the same commit** as the code they describe. Non-trivial design decisions
  get an ADR ([docs/adr/](docs/adr/README.md), template `0000-template.md`).
- **Changelog.** A change users would notice adds a fragment, `changelog.d/<branch>.md`, in
  the same commit; don't edit CHANGELOG.md itself (parallel PRs would conflict on it). Format:
  [changelog.d/README.md](changelog.d/README.md). What counts and how releases are cut:
  [docs/dev/RELEASING.md](docs/dev/RELEASING.md).
- **Dependencies:** pin exact versions, check the licence is in the `deny.toml` allow list
  (adding a licence needs an ADR), and verify the crate's API before using it.
- **Never commit** secrets (`.env`, `.env.local`), certificates, audio recordings, models or
  files over 5 MB, and never hand-edit generated files (`app/*.xcodeproj`,
  `app/RustBridge/generated/`). No network code and no telemetry, ever.

## Where things live

See the map in [AGENTS.md](AGENTS.md) and the documentation index in
[docs/README.md](docs/README.md). The deep dives explain the audio engineering for readers who
know programming but are new to DSP.

## AI coding agents

Much of hEARo has been written together with AI coding agents, and the repository is set up
for that: [AGENTS.md](AGENTS.md) holds the instructions every agent (and human) follows,
[CLAUDE.md](CLAUDE.md) adds Claude Code specifics, and `.claude/` contains shared project
settings, hooks (block edits to generated files and hook bypasses, format edited files), review subagents
(real-time safety, DSP, ML, SwiftUI, docs, test gaps) and skills (new DSP module, ADRs, ML
experiments, releases, audio debugging). [docs/PROGRESS.md](docs/PROGRESS.md) and
[docs/OPEN_QUESTIONS.md](docs/OPEN_QUESTIONS.md) are the agents' working log and their queue
of questions for the maintainer. Contributions made with AI help are welcome under the same
rules as any other: you are responsible for understanding and testing what you submit.

## Licence of contributions

hEARo is dual-licensed under MIT OR Apache-2.0. Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in hEARo by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any additional terms or
conditions.
