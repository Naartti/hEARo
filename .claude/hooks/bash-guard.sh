#!/usr/bin/env bash
# PreToolUse(Bash): block hook bypasses and force pushes; ask before gh leaves this repo.
# Tests: scripts/test-hooks.sh (run by `just check`).
set -euo pipefail
input="$(cat)"
cmd="$(python3 "$(dirname "$0")/_input.py" tool_input.command <<<"$input")"
if [[ "$cmd" =~ (^|[[:space:]])--no-verify([[:space:]]|$) ]]; then
  echo "Blocked: --no-verify skips the local CI hooks. Fix the failing check instead." >&2; exit 2
fi
if [[ "$cmd" =~ git[[:space:]]+push.*(--force|[[:space:]]-f([[:space:]]|$)|--force-with-lease) ]]; then
  echo "Blocked: force push. Ask the human." >&2; exit 2
fi
cwd="$(python3 "$(dirname "$0")/_input.py" cwd <<<"$input" || true)"
# Only real commits (start of command or after ; & | newline), not commands that mention one;
# `git -C <dir> commit` too. Judged in the checkout the commit happens in: a worktree has its own
# tree and its own target/ci/check.ok (CLAUDE_PROJECT_DIR is the main checkout).
if [[ "$cmd" =~ (^|[\;\&\|$'\n'])[[:space:]]*git([[:space:]]+-C[[:space:]]*([^[:space:]\;\&\|]+))?[[:space:]]+commit([[:space:]]|$) ]]; then
  dir="${BASH_REMATCH[3]//[\"\']/}"
  base="${cwd:-$PWD}"
  # `cd <dir>` first: the commit runs there.
  if [[ "$cmd" =~ ^[[:space:]]*cd[[:space:]]+([^[:space:]\;\&\|]+) ]]; then
    c="${BASH_REMATCH[1]//[\"\']/}"
    if [[ "$c" == /* ]]; then base="$c"; else base="$base/$c"; fi
  fi
  if [[ -n "$dir" ]]; then
    if [[ "$dir" == /* ]]; then base="$dir"; else base="$base/$dir"; fi
  fi
  root="$(git -C "$base" rev-parse --show-toplevel 2>/dev/null || echo "${CLAUDE_PROJECT_DIR:-$PWD}")"
  want="$(cat "$root/target/ci/check.ok" 2>/dev/null || true)"
  # No fingerprint script there (another repo, or none): fail closed, never an empty match.
  have="$("$root/scripts/tree-fingerprint.sh" 2>/dev/null || echo "no fingerprint script in $root")"
  if [[ "$want" != "$have" ]]; then
    echo "Blocked: the working tree changed since the last green \`just check\`. Run \`just check\`, then commit (git add + commit don't change the fingerprint)." >&2
    exit 2
  fi
fi
# Last: prints a PreToolUse "ask" decision (JSON on stdout) when gh may reach another repo.
# If the check itself fails, ask rather than let an auto-allowed command through unchecked.
root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}"
if ! out="$(python3 "$(dirname "$0")/gh_scope.py" "${cwd:-$PWD}" "$root" <<<"$cmd")"; then
  out='{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": "gh scope check failed"}}'
fi
[[ -n "$out" ]] && echo "$out"
exit 0
