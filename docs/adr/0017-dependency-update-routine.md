# 0017. Dependency updates: weekly agent routine, agent merges green Dependabot bumps

Date: 2026-11-15 · Status: accepted

## Context

ADR 0016 turned on Dependabot for GitHub Actions and Cargo (weekly, grouped, 14-day cooldown);
this adds `uv` for `ml/`. Each PR runs `just ci` on a hosted Mac, but hosted runs skip the CPU
budgets, and live audio is only testable on a real Mac. The owner wants Claude Code to handle
these PRs on its own and only hear about what needs a decision. AGENTS.md says the human
merges, so the exception needs a written rule. `main` is protected by a ruleset that requires
the `just ci` check (set up by the owner).

## Decision

We handle dependency updates with a weekly agent routine (the `dependency-updates` skill). The
agent merges a Dependabot PR itself only when the diff touches nothing but manifests, lockfiles
or workflow pins, it is a grouped patch/minor (or security patch/minor) update, hosted checks
are green, and, for Cargo, a local `just ci` (golden tests + CPU budgets) is green. One merge at
a time, waiting for `main` to go green. Majors, red runs and anything needing code changes go
through the normal workflow with the human merging; updates not worth doing now are ignored via
`@dependabot ignore` and recorded in PROGRESS.md. A breaking merge is reverted first, then fixed.

## Alternatives considered

- **Human reviews every PR** — the checks are mechanical; reading a green patch bump adds
  nothing but delay, so updates pile up.
- **GitHub auto-merge on green** — skips the local CPU-budget run and the release-note read,
  and would merge while nobody is watching `main`.
- **Renovate** — also covers `.mise.toml`, but another bot and config for a small gap.

## Consequences (incl. how we'd know it was wrong)

- Routine bumps cost a weekly agent run instead of human time; the human sees a report and only
  the PRs that need judgement.
- `.mise.toml` toolchains and Swift (no packages yet) are still bumped by hand.
- Wrong if: an agent-merged bump is reverted more than once a quarter, or a regression from one
  reaches a release — then require human approval for Cargo groups too.
