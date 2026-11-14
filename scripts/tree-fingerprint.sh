#!/usr/bin/env bash
# Print a fingerprint of the working tree: the git tree hash of every tracked and untracked,
# non-ignored file, built in a throwaway index so the real staging area is untouched and doesn't
# matter. `just check`/`just ci` record it on success in target/ci/check.ok; the Claude Bash hook
# refuses `git commit` when the tree no longer matches, i.e. the gate didn't run on this code.
set -euo pipefail
cd "$(dirname "$0")/.."
idx="$(mktemp)"
trap 'rm -f "$idx"' EXIT
cp "$(git rev-parse --git-path index)" "$idx" 2>/dev/null || true
GIT_INDEX_FILE="$idx" git add -A
GIT_INDEX_FILE="$idx" git write-tree
