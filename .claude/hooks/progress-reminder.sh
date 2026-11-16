#!/usr/bin/env bash
# Stop: remind to record the task in docs/progress.d/ when code changed and no progress note
# changed with it. Feature branches never edit docs/PROGRESS.md itself (parallel PRs would
# conflict; docs/progress.d/README.md).
cd "$CLAUDE_PROJECT_DIR" 2>/dev/null || exit 0
git rev-parse --git-dir >/dev/null 2>&1 || exit 0
changed="$(git status --porcelain -- crates app ml scripts 2>/dev/null | head -1)"
progress="$(git status --porcelain -- docs/progress.d 2>/dev/null)"
if [[ -n "$changed" && -z "$progress" ]]; then
  echo "Reminder: code changed but no task note was written. Add docs/progress.d/<branch>.md (not PROGRESS.md)." >&2
fi
exit 0
