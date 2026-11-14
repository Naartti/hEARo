@AGENTS.md

## Claude Code specifics

- Work autonomously through docs/PROGRESS.md; stop only for items in docs/OPEN_QUESTIONS.md.
- Run `just check` before every commit and `just ci` at milestone end (see "Local CI" above).
  Hooks in `.claude/settings.json` format edited files and block edits to generated files.
- Use subagents for reviews: rt-safety-reviewer before committing any change to RT code;
  dsp-reviewer for new modules; docs-keeper and test-gap-finder at milestone end.
- Use skills: new-dsp-module, write-adr, ml-experiment, release, debug-audio.
- Prefer `just` recipes over raw commands so hooks and permissions stay simple.
