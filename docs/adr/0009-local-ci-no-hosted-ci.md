# 0009. Local CI only — no hosted CI

Date: 2026-11-13 · Status: superseded by [0016](0016-github-actions-ci-and-releases.md)
(it superseded the GitHub Actions plan in HANDOVER §10.7)

## Context

This is a personal project; the owner does not want to pay for hosted macOS CI minutes.

## Decision

`just ci` (`scripts/ci.sh`) is the CI: format, lint, all tests, golden tests, CPU budgets,
cargo-deny, cargo-machete, Swift build/tests, XcodeGen spec, Python lint/tests, docs lint and
offline link checks. It logs to `target/ci/last.log`. Lefthook enforces it: pre-commit runs
fast staged checks, pre-push runs `just ci`. Agents run `just check` before every commit and
`just ci` at milestone end (AGENTS.md). `.github/workflows/` is blocked by the repo hygiene check
and the Claude edit guard.

## Alternatives considered

- **GitHub Actions on macOS runners** — costs money; slower feedback than local M-series runs.
- **Self-hosted runner on this Mac** — extra moving parts for no gain with one developer.

## Consequences (incl. how we'd know it was wrong)

- No "clean machine" verification; mitigated by `just bootstrap && just ci` from a fresh clone
  at each milestone end.
- Wrong if: environment drift causes "works on my machine" failures after OS/tool updates —
  then add a periodic fresh-clone run in a temp dir (`scripts/ci-fresh.sh`).
