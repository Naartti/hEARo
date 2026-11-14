# CI

The checks are defined once, in `scripts/`, and run through `just`: on your Mac before every
commit and push, and again in GitHub Actions on every pull request
([ADR 0016](../adr/0016-github-actions-ci-and-releases.md), which superseded the local-only
[ADR 0009](../adr/0009-local-ci-no-hosted-ci.md)).

## The two gates

| Gate | Script | When | Typical time (warm / cold) |
|---|---|---|---|
| `just check` | `scripts/check.sh` | before every commit | ≈5 s / ≈60 s |
| `just ci` | `scripts/ci.sh` | before push (hook), milestone end, dependency/toolchain bumps; in GitHub Actions on every PR | ≈10 s / ≈80 s locally |

`just check`: rustfmt, clippy (`-D warnings`), every fast Rust test via nextest (unit,
property, RT-safety, 10-min stress, CLI end-to-end), Swift build (+ tests with Xcode), ruff +
pytest (fast), repo hygiene on staged files.

`just ci` adds: golden audio tests, doc tests, benchmarks compile, CPU budgets (release build), cargo-deny
(licenses/advisories/sources), cargo-machete (unused deps), XcodeGen spec generation,
markdownlint, offline link check, hygiene over all tracked files. Output is tee'd to
`target/ci/last.log`. Each step reports ✔/✘ with timing; all steps run even if one fails, and
the summary lists failures.

## GitHub Actions

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` → *just ci* | every PR, push to `main` | `just ci` on `macos-26` with the `.mise.toml` toolchains. CPU budgets are skipped (`GITHUB_ACTIONS=true`): shared-runner timing is noise, so run `just bench` on a Mac after DSP changes. On failure the log is attached as the `ci-log` artifact. |
| `ci.yml` → *coverage* | every PR, push to `main` | `just coverage`; summary in the job summary, `lcov.info` uploaded to Codecov (OIDC, no secret) |
| `release.yml` | tag `v*`, or manually as a dry run | builds and publishes a release (see [Releasing](#releasing)) |

Rules: a workflow only calls `just`/`scripts/` — anything it runs must also run locally.
Actions are pinned to a full commit SHA with the version as a comment (`scripts/check-files.sh`
rejects anything else); Dependabot proposes grouped updates weekly with a 14-day cooldown.
Workflows default to read-only permissions; jobs ask for more only where they need it.

Not covered by hosted CI: hardware tests (`just test-hw`), the live Core Audio tap and CPU
budgets. Those stay on a real Mac (docs/testing/HARDWARE_CHECKLIST.md).

## Coverage

`just coverage` runs the fast Rust test suite under `cargo-llvm-cov` and writes
`target/coverage/lcov.info`, `summary.txt` and `html/index.html`. Engine code that needs real
Core Audio shows as uncovered because its hardware tests are `#[ignore]`d. Swift coverage is
not measured yet.

## Releasing

Bump the version (the `release` skill has the checklist), merge to `main`, then tag:
`git tag v0.2.0 && git push origin v0.2.0`. `release.yml` checks the tag matches `Cargo.toml`,
builds with `scripts/build-app.sh release`, signs **ad-hoc** (`ALLOW_ADHOC=1 SIGN_IDENTITY=-`),
packages `hEARo-<version>.dmg`, writes a `.sha256`, attests build provenance and publishes the
GitHub Release with generated notes (`.github/release.yml` groups them by label). Ad-hoc keeps
the personal signing certificate, which carries the owner's name, out of public binaries.
`just release` still builds a personally signed DMG for your own Macs; don't upload that one.

## Hooks (lefthook)

| Hook | Runs |
|---|---|
| pre-commit | rustfmt check, ruff, markdownlint on staged files; size/audio/secret/pinning hygiene |
| commit-msg | Conventional Commits check |
| pre-push | `just ci` |

Install with `lefthook install` (bootstrap does it). Don't skip with `--no-verify`.

**Agent commit gate.** A green `just check` or `just ci` writes the working-tree fingerprint
(`scripts/tree-fingerprint.sh`, a git tree hash of all non-ignored files) to
`target/ci/check.ok`. The Claude Code Bash hook (`.claude/hooks/bash-guard.sh`) refuses
`git commit` when the tree no longer matches, so agents can't commit code the gate never saw.
Staging doesn't change the fingerprint. Humans aren't affected.

## Other useful targets

`just test-golden`, `just bench`, `just coverage`, `just docs`, `just deny`, `just ml-check`,
`just lint`, `just fmt`.

## Fresh-clone verification (milestone end)

```sh
tmp=$(mktemp -d) && git clone . "$tmp/hearo" && cd "$tmp/hearo" && just bootstrap && just ci
```
