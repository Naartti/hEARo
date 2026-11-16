@AGENTS.md

## Claude Code specifics

- Work autonomously through docs/PROGRESS.md (+ docs/progress.d/); stop only for items in
  docs/OPEN_QUESTIONS.md. Feature branches record tasks in progress.d/ and changelog.d/, never
  in PROGRESS.md or CHANGELOG.md (parallel PRs would conflict; `just check` enforces it).
- Run `just check` before every commit and `just ci` at milestone end (see "CI" above).
  The Bash hook blocks `git commit` if the tree changed since the last green check.
- Change files with Edit/Write, not shell heredocs or scripts: the hooks in
  `.claude/settings.json` that format edited files and block edits to generated files only see
  Edit/Write.
- Follow "Any change" in AGENTS.md for every request, including one-line prompts.
- Reviews (step 6) by area touched — run the independent ones in parallel, give them the diff
  range, not your conclusions:
  - RT code → rt-safety-reviewer (before *each* commit touching it), effort `high`
  - DSP module added/reworked → dsp-reviewer, effort `high`
  - `app/` UI → swift-ui-reviewer, effort `medium`
  - `ml/`, `crates/hearo-ml` → ml-reviewer, effort `high`
  - every branch → the `code-review` skill at level `medium` on `main...HEAD`
  - large change or milestone end → test-gap-finder; milestone end → docs-keeper
- Token discipline: Explore subagent for broad searches; don't re-read files you just edited;
  don't spawn agents for a handful of tool calls. Reviewer models are pinned in
  `.claude/agents/*.md` (`opus` where correctness is subtle, `sonnet` otherwise).
- Use skills: new-dsp-module, write-adr, ml-experiment, release, debug-audio,
  dependency-updates (weekly Dependabot pass).
- Prefer `just` recipes over raw commands so hooks and permissions stay simple.
