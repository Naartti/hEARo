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
pytest (fast), repo hygiene on staged files, agent-hook tests (`scripts/test-hooks.sh`).

`just ci` adds: golden audio tests, doc tests, benchmarks compile, CPU budgets (release build), cargo-deny
(licenses/advisories/sources), cargo-machete (unused deps), XcodeGen spec generation,
actionlint (workflow syntax, expressions and shellcheck of `run:` blocks), markdownlint,
offline link check, hygiene over all tracked files. Output is tee'd to
`target/ci/last.log`. Each step reports ✔/✘ with timing; all steps run even if one fails, and
the summary lists failures.

## GitHub Actions

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` → *just ci* | every PR, push to `main` | `just ci` on `macos-26` with the `.mise.toml` toolchains and **Xcode 26.6**, the oldest we support (`swift-tools-version: 6.2`) — newer Swift accepts code that 26.x rejects, so a green local build on a newer Xcode isn't enough (`XCODE_VERSION` in both workflows). CPU budgets are skipped (`GITHUB_ACTIONS=true`): shared-runner timing is noise, so run `just bench` on a Mac after DSP changes. On failure the log is attached as the `ci-log` artifact. |
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

Bump the version (the `release` skill has the checklist), merge to `main`, wait for CI on
`main` to go green, then tag: `git tag v0.2.0 && git push origin v0.2.0`. `release.yml`'s
read-only **build** job refuses a tag that doesn't match `Cargo.toml`, isn't on `main` or has no
successful CI run, then builds with `scripts/build-app.sh release`, signs **ad-hoc**
(`ALLOW_ADHOC=1 SIGN_IDENTITY=-`), packages `hEARo-<version>.dmg` and writes a `.sha256`. Only
the **publish** job (tags only) gets write permissions: it attests build provenance and
creates the GitHub Release with generated notes (`.github/release.yml` groups them by label).
A pre-release version (`0.2.0-rc.1`) becomes a GitHub pre-release and never "latest". Ad-hoc
keeps the personal signing certificate, which carries the owner's name, out of public
binaries. `just release` still builds a personally signed DMG for your own Macs; don't upload
that one.

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

**Agent `gh` scope.** `.claude/settings.json` auto-allows some `gh` commands by prefix, and a
prefix can't see which repository `--repo`/`-R`, `GH_REPO`, `GH_HOST`, `--hostname`, a URL or a
`gh api` path selects. The same Bash hook (`.claude/hooks/gh_scope.py`) scans the command text
(quotes and line continuations removed) and, if it mentions `gh` and anything in it points
beyond `Naartti/hEARo`, returns a PreToolUse **ask** decision: the command becomes a normal
permission prompt instead of running. It also asks when the command uses `$`/backtick
expansion, changes directory (`cd`, `git -C`, `GIT_DIR`) or runs outside the project, for
mutating `gh api` calls (`-X` other than GET, `-f`/`-F`/`--input`, GraphQL), and for `gh pr
checkout` of a PR not authored by Dependabot or the owner, since `just ci` would run that PR's
code. If the check fails it asks too. It never blocks, so a false positive (say, a link in a
comment body) costs one prompt. Commands that match no allow rule (`command gh …`, `bash -c …`,
`git fetch`, `git checkout`) prompt anyway.

## Other useful targets

`just test-golden`, `just bench`, `just coverage`, `just docs`, `just deny`, `just ml-check`,
`just lint`, `just fmt`.

## Fresh-clone verification (milestone end)

```sh
tmp=$(mktemp -d) && git clone . "$tmp/hearo" && cd "$tmp/hearo" && just bootstrap && just ci
```
