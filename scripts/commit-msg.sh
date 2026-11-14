#!/usr/bin/env bash
# Conventional Commits check for the commit-msg hook.
set -euo pipefail
msg="$(head -n1 "$1")"
re='^(feat|fix|docs|test|refactor|perf|build|chore|ci|style|revert)(\([a-z0-9._-]+\))?!?: .{1,100}$'
if [[ "$msg" =~ ^(Merge|Revert|fixup!|squash!) ]] || [[ "$msg" =~ $re ]]; then exit 0; fi
echo "✘ Commit message must follow Conventional Commits, e.g. 'feat(dsp): add de-hum notch comb'"
echo "  got: $msg"
exit 1
