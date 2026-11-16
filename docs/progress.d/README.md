# progress.d — task notes since the last roll-up

[PROGRESS.md](../PROGRESS.md) is the plan; this folder is what landed since it was last rolled
up. Each PR that finishes (or adds) a task writes one file here, named after its branch with
`/` as `-` (`feat/sidecar-live-settings` → `feat-sidecar-live-settings.md`), instead of editing
PROGRESS.md: two PRs ticking neighbouring lines there always conflicted (2026-11-16).

```markdown
# Sidecars record live settings (M4, owner's plan step 3a)

Closes in PROGRESS.md: "Auto-save sidecars list every slot in `modules` …"

Acceptance: …  Done: …  Decisions: …  Review notes: fixed … / rejected … because …

## Follow-ups

- [ ] Something found on the way (becomes a task in PROGRESS.md at the roll-up).
```

- A session starts by reading PROGRESS.md **and** every file here.
- A later PR that changes the same task edits that task's file, never another PR's.
- The roll-up (`docs/rollup-<date>` branch; docs-keeper at milestone end, or when this folder
  passes ~10 files) ticks PROGRESS.md from these files, moves the follow-ups into it and
  deletes the files. Only roll-up and release branches may edit PROGRESS.md and CHANGELOG.md
  (`just check` enforces it, `scripts/changelog.py`).
