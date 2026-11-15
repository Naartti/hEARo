---
name: dependency-updates
description: Weekly routine for Dependabot PRs and alerts - triage, verify, merge the safe ones, fix or defer the rest, report. Use when asked to handle Dependabot/dependency PRs, on a security alert, or for the weekly dependency pass.
---

# Dependency updates (ADR 0019)

Run weekly (or right away for a security alert). The agent may merge a Dependabot PR **itself**
when it is in class A below and every gate is green; anything where the agent wrote code goes
through the normal "Any change" workflow and the human merges. Never bypass a gate, never
force-push, never change repo settings.

## 0. Preconditions

1. Clean tree; `git switch main && git pull --ff-only`.
2. `main` is green: `gh run list --workflow ci.yml --branch main --limit 1`. If it is red, fix
   that first (normal workflow) — never merge updates onto a red `main`.

## 1. Inventory

```sh
gh pr list --author app/dependabot --json number,title,headRefName,mergeable,statusCheckRollup,files
gh api repos/Naartti/hEARo/dependabot/alerts --jq '.[] | select(.state=="open") |
  {number, severity: .security_advisory.severity, pkg: .dependency.package.name, fix: .security_vulnerability.first_patched_version.identifier}'
```

Order: security fixes (any ecosystem) → GitHub Actions → Cargo patch/minor group → uv → Cargo
majors. Merge one PR at a time; Dependabot rebases the rest.

## 2. Classify each PR

**A — merge after checks.** The diff only touches manifests, lockfiles or workflow `uses:` pins,
and it is a grouped (`actions`, `cargo-patch-minor`, `uv`) or security update where **every**
bumped dependency is semver-compatible. Check each line of the PR's version table, not the
group name:

- `x.y.z → x.y'.z'` with `x ≥ 1`: compatible. Any major change: class B.
- `0.y.z → 0.y'.z`: a **breaking** change in semver for 0.x (Cargo treats it so too) — class B,
  even though Dependabot calls it "minor". `0.y.z → 0.y.z'` is compatible.
- A crate an ADR is built on (`uniffi*`, `objc2*`, `tract*`/`ort*`, `hound`, `realfft`) at any
  non-patch bump: class B, and "Stop and ask" if it changes that ADR's decision.

Then:

- Hosted checks green (`gh pr checks <n>`). `just ci` is required; `coverage` red only blocks
  if the failure is in the tests, not the Codecov upload.
- Diff is what the title says: `gh pr diff <n>`. Cargo pins stay exact (`=x.y.z`); workflow
  pins stay `@<sha> # vX.Y.Z` (the hygiene check enforces it); no new workflow steps.
- Release notes in the PR body: look for *breaking*, *MSRV*, *yanked*, *security*, behaviour
  changes. Anything alarming → class B.
- **Cargo only**: hosted runners skip the CPU budgets, so run them here:
  `gh pr checkout <n> && just ci` (includes golden tests and budgets). Then `git switch main`.
- A new transitive crate is checked for licence/advisories by cargo-deny in `just ci`; also
  confirm it isn't reachable from RT code doing allocation/locking (`cargo tree -i <crate>`);
  if it is in `hearo-dsp`/`hearo-engine`'s tree and new, run rt-safety-reviewer (effort high)
  on the PR diff before merging.

Merge: `gh pr merge <n> --rebase`. Then `gh run watch` the CI run on `main`; it must go green
before the next merge.

**B — needs work.** Major version, red CI, alarming release notes, or a change that needs
source edits. Do it with the "Any change" workflow (AGENTS.md): branch
`build/<crate>-<version>` from `main`, bump the same version, fix code test-first, `just
check` per commit, area reviewers for what the fix touches (rt-safety-reviewer / dsp-reviewer /
swift-ui-reviewer / ml-reviewer), `just ci`, PR with How to verify. The human merges it;
Dependabot closes its own PR once `main` contains the version. Don't push commits onto
Dependabot's branch (it then stops rebasing it).

**C — defer.** The fix is out of proportion now, or upstream is broken. Comment
`@dependabot ignore this major version` (or `... minor version` / `... dependency`) with the
reason, and add a line under *Dependency notes* in docs/PROGRESS.md Backlog (what, why, when to
retry). Undo later with `@dependabot unignore <dep>`.

## 3. Housekeeping

- Conflicted or stale PR: comment `@dependabot rebase`, then wait for its checks.
- Flaky hosted run: re-run once (`gh run rerun <id> --failed`). Red twice = real; class B.
- Security alert without a PR: if a fixed version exists, do it as class B (or A if it's a
  lockfile-only patch bump: `cargo update -p <crate> --precise <ver>` / `uv lock -P <pkg>`).
  If no fix exists, **stop and tell the human** (severity, whether hEARo reaches the code).

## 4. If something breaks after a merge

1. Red `main`: `git switch -c revert/<dep>-<version>`, `git revert <sha>`, `just check`, push,
   PR; the agent may merge a pure revert once green (it restores a tested state).
2. Comment `@dependabot ignore this <major|minor> version` on the reopened PR (or note it for
   the next run), record it under *Dependency notes*, then treat it as class B.
3. Breakage only visible live (audio, permissions): `debug-audio` skill first; same revert path.

## 5. Report

End with: merged (PR numbers + versions), PRs opened for the human, ignored/deferred with
reasons, open security alerts, and a *How to verify* list only if a dependency of
`hearo-engine`, `hearo-ffi` or the app changed (`just app`, play audio, toggle bypass, switch
output device).

## Stop and ask the human

A security alert with no fixed version · a major bump that changes a decision in an ADR (e.g.
UniFFI, objc2-core-audio) · a new licence `deny.toml` doesn't allow · anything needing repo
settings, secrets or a force-push.
