#!/usr/bin/env bash
# Stop: remind to update docs/PROGRESS.md when code changed since it was last touched.
cd "$CLAUDE_PROJECT_DIR" 2>/dev/null || exit 0
git rev-parse --git-dir >/dev/null 2>&1 || exit 0
changed="$(git status --porcelain -- crates app ml scripts 2>/dev/null | head -1)"
progress="$(git status --porcelain -- docs/PROGRESS.md 2>/dev/null)"
if [[ -n "$changed" && -z "$progress" ]]; then
  echo "Reminder: code changed but docs/PROGRESS.md was not updated. Tick the task and add a note." >&2
fi
exit 0
