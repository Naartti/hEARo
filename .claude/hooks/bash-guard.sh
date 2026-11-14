#!/usr/bin/env bash
# PreToolUse(Bash): block hook bypasses and force pushes.
set -euo pipefail
cmd="$(python3 "$(dirname "$0")/_input.py" tool_input.command)"
if [[ "$cmd" =~ (^|[[:space:]])--no-verify([[:space:]]|$) ]]; then
  echo "Blocked: --no-verify skips the local CI hooks. Fix the failing check instead." >&2; exit 2
fi
if [[ "$cmd" =~ git[[:space:]]+push.*(--force|[[:space:]]-f([[:space:]]|$)|--force-with-lease) ]]; then
  echo "Blocked: force push. Ask the human." >&2; exit 2
fi
# Only real commits (start of command or after ; & | newline), not commands that mention one.
if [[ "$cmd" =~ (^|[\;\&\|$'\n'])[[:space:]]*git[[:space:]]+commit ]]; then
  root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}"
  want="$(cat "$root/target/ci/check.ok" 2>/dev/null || true)"
  have="$("$root/scripts/tree-fingerprint.sh")"
  if [[ "$want" != "$have" ]]; then
    echo "Blocked: the working tree changed since the last green \`just check\`. Run \`just check\`, then commit (git add + commit don't change the fingerprint)." >&2
    exit 2
  fi
fi
exit 0
