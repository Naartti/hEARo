# Local CI

hEARo has no hosted CI ([ADR 0009](../adr/0009-local-ci-no-hosted-ci.md)). The checks run on
your Mac through `just`, implemented in `scripts/`.

## The two gates

| Gate | Script | When | Typical time (warm / cold) |
|---|---|---|---|
| `just check` | `scripts/check.sh` | before every commit | ≈5 s / ≈60 s |
| `just ci` | `scripts/ci.sh` | before push, milestone end, dependency/toolchain bumps | ≈10 s / ≈80 s |

`just check`: rustfmt, clippy (`-D warnings`), every fast Rust test via nextest (unit,
property, RT-safety, 10-min stress, CLI end-to-end), Swift build (+ tests with Xcode), ruff +
pytest (fast), repo hygiene on staged files.

`just ci` adds: golden audio tests, doc tests, benchmarks compile, CPU budgets (release build), cargo-deny
(licenses/advisories/sources), cargo-machete (unused deps), XcodeGen spec generation,
markdownlint, offline link check, hygiene over all tracked files. Output is tee'd to
`target/ci/last.log`. Each step reports ✔/✘ with timing; all steps run even if one fails, and
the summary lists failures.

## Hooks (lefthook)

| Hook | Runs |
|---|---|
| pre-commit | rustfmt check, ruff, markdownlint on staged files; size/audio/secret hygiene |
| commit-msg | Conventional Commits check |
| pre-push | `just ci` |

Install with `lefthook install` (bootstrap does it). Don't skip with `--no-verify`.

**Agent commit gate.** A green `just check` or `just ci` writes the working-tree fingerprint
(`scripts/tree-fingerprint.sh`, a git tree hash of all non-ignored files) to
`target/ci/check.ok`. The Claude Code Bash hook (`.claude/hooks/bash-guard.sh`) refuses
`git commit` when the tree no longer matches, so agents can't commit code the gate never saw.
Staging doesn't change the fingerprint. Humans aren't affected.

## Other useful targets

`just test-golden`, `just bench`, `just docs`, `just deny`, `just ml-check`, `just lint`, `just fmt`.

## Fresh-clone verification (milestone end)

```sh
tmp=$(mktemp -d) && git clone . "$tmp/hearo" && cd "$tmp/hearo" && just bootstrap && just ci
```
