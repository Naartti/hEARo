#!/usr/bin/env bash
# Tests for the Claude Code hooks in .claude/hooks (run by `just check`).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
guard="$root/.claude/hooks/bash-guard.sh"
fail=0

run_guard() { # $1 = command; prints the guard's exit code
  local json
  json="$(python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$1")"
  local code=0
  CLAUDE_PROJECT_DIR="$root" "$guard" <<<"$json" >/dev/null 2>&1 || code=$?
  echo "$code"
}
expect() { # $1 = allow|block, $2 = command
  local code want
  code="$(run_guard "$2")"
  if [[ "$1" == allow ]]; then want=0; else want=2; fi
  if [[ "$code" != "$want" ]]; then
    echo "FAIL ($1, got exit $code): $2" >&2
    fail=1
  fi
}

# gh may only target this repository (the allow rules match by prefix and can't see the repo).
expect allow 'gh pr list --author app/dependabot --json number,title'
expect allow 'gh pr view 3 --repo Naartti/hEARo'
expect allow 'gh pr list -R naartti/hearo'
expect allow 'gh pr list -RNaartti/hEARo'
expect allow 'gh pr view https://github.com/Naartti/hEARo/pull/3'
expect allow 'gh pr comment 3 --body "see https://github.com/rust-lang/rust/issues/1"'
expect allow 'gh pr comment 3 -b https://github.com/rust-lang/rust/issues/1'
expect allow 'gh pr merge 3 -r'
expect allow 'gh api --paginate "repos/Naartti/hEARo/dependabot/alerts?state=open"'
expect allow 'gh pr checkout 3 && just ci'
expect allow 'ls -R docs && gh pr list'
expect allow 'echo "gh is great" --repo x/y'
expect block 'gh pr view 12 --repo someone/else'
expect block 'gh pr view 12 --repo=someone/else'
expect block 'gh pr list -R someone/else'
expect block 'gh pr list -Rsomeone/else'
expect block 'gh run view 1 --repo someone/hEARo'
expect block 'gh pr list -R naartti/hearo-fork'
expect block 'GH_REPO=someone/else gh pr list'
expect block 'env GH_HOST=ghe.example.com gh pr list'
expect block 'gh pr comment https://github.com/someone/else/pull/1 --body hi'
expect block 'gh pr view --web https://github.com/someone/else/pull/1'
expect block 'gh pr checks 3; gh pr comment 1 -R someone/else --body x'
expect block 'cd /tmp && gh pr list --repo someone/else'

# Existing rules still hold.
expect block 'git commit --no-verify -m x'
expect block 'git push --force origin main'

if [[ "$fail" != 0 ]]; then exit 1; fi
echo "hook tests passed"
