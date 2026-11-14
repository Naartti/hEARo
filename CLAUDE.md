@AGENTS.md

## Claude Code specifics

- Work autonomously through docs/PROGRESS.md; stop only for items in docs/OPEN_QUESTIONS.md.
- Run `just check` before every commit and `just ci` at milestone end (see "Local CI" above).
  Hooks in `.claude/settings.json` format edited files and block edits to generated files.
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
- Use skills: new-dsp-module, write-adr, ml-experiment, release, debug-audio.
- Prefer `just` recipes over raw commands so hooks and permissions stay simple.
