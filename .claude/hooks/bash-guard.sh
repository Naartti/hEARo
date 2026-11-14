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
exit 0
