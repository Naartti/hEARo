#!/usr/bin/env bash
# Tests for the Claude Code hooks in .claude/hooks (run by `just check`).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
guard="$root/.claude/hooks/bash-guard.sh"
fail=0

# A fake `gh` for the PR-author lookup in `gh pr checkout`: prints $FAKE_PR_AUTHOR.
fakebin="$(mktemp -d)"
trap 'rm -rf "$fakebin"' EXIT
printf '#!/bin/sh\nprintf "%%s\\n" "$FAKE_PR_AUTHOR"\n' >"$fakebin/gh"
chmod +x "$fakebin/gh"
export PATH="$fakebin:$PATH" FAKE_PR_AUTHOR=""
CWD="$root"

outcome() { # $1 = command → allow | ask | block   (hook cwd = $CWD)
  local json out code=0
  json="$(python3 -c 'import json,sys; print(json.dumps({"cwd": sys.argv[2], "tool_input": {"command": sys.argv[1]}}))' "$1" "$CWD")"
  out="$(CLAUDE_PROJECT_DIR="$root" "$guard" <<<"$json" 2>/dev/null)" || code=$?
  if [[ "$code" == 2 ]]; then echo block
  elif [[ "$code" != 0 ]]; then echo "error($code)"
  elif [[ "$out" == *'"permissionDecision": "ask"'* ]]; then echo ask
  else echo allow; fi
}
expect() { # $1 = allow|ask|block, $2 = command
  local got
  got="$(outcome "$2")"
  if [[ "$got" != "$1" ]]; then
    echo "FAIL (want $1, got $got): $2" >&2
    fail=1
  fi
}

# gh on this repository: stays auto-allowed.
expect allow 'gh pr list --author app/dependabot --json number,title'
expect allow 'gh pr view 3 --repo Naartti/hEARo'
expect allow 'gh pr list -R naartti/hearo'
expect allow 'gh pr list -RNaartti/hEARo'
expect allow 'gh pr view https://github.com/Naartti/hEARo/pull/3'
expect allow 'gh pr merge 3 -r'
expect allow 'gh api --paginate "repos/Naartti/hEARo/dependabot/alerts?state=open" --jq ".[]"'
expect allow 'gh api repos/Naartti/hEARo/rules/branches/main -X GET'
expect allow 'gh pr checks 3 && gh run list --branch main --limit 1'
expect allow $'gh pr comment 3 --body-file - <<EOF\ndon\'t merge yet\nEOF'
expect allow 'echo "no github cli here" --repo x/y'

# Pointing somewhere else: back to a prompt.
expect ask 'gh pr view 12 --repo someone/else'
expect ask 'gh pr view 12 --repo=someone/else'
expect ask 'gh pr list -R someone/else'
expect ask 'gh pr list -Rsomeone/else'
expect ask 'gh run view 1 --repo someone/hEARo'
expect ask 'gh pr list -R naartti/hearo-fork'
expect ask 'gh pr list --repo naartti/hearo/x'
expect ask 'GH_REPO=someone/else gh pr list'
expect ask 'env GH_HOST=ghe.example.com gh pr list'
expect ask 'gh api --hostname ghe.example.com repos/Naartti/hEARo'
expect ask 'gh pr comment https://github.com/someone/else/pull/1 --body hi'
expect ask 'gh pr merge -m https://github.com/someone/else/pull/1'
expect ask 'gh pr comment 3 --body "see https://github.com/rust-lang/rust/issues/1"'
expect ask 'gh pr checks 3; gh pr comment 1 -R someone/else --body x'
expect ask 'cd /tmp && gh pr list --repo someone/else'
expect ask '/opt/homebrew/bin/gh pr list -R someone/else'
expect ask $'gh pr list \\\n  -R someone/else'
expect ask 'gh pr list 2>&1 -R someone/else'
expect ask 'command gh pr list -R someone/else'
expect ask 'if gh pr view 1 -R someone/else; then :; fi'
expect ask 'echo `gh pr view 1 -R someone/else`'
expect ask 'ls -R docs && gh pr list'
expect ask 'gh api repos/someone/else/issues/1/comments'
expect ask 'gh api repos/Naartti/hEARo/git/refs/heads/main -X DELETE'
expect ask 'gh api --method=put repos/Naartti/hEARo/pulls/5/merge'
expect ask 'gh api repos/Naartti/hEARo/issues/1/comments -f body=x'
expect ask 'gh api graphql -f query=x'
expect ask 'gh pr checkout some-branch'
# Quoting, expansion and directory tricks.
expect ask "gh pr list --re''po someone/else"
expect ask 'gh pr list -"R" someone/else'
expect ask "export GH_R''EPO=someone/else; gh pr list"
expect ask "gh pr view https://github.com/some''one/else/pull/1"
expect ask 'r=someone/else; gh pr list --repo $r'
expect ask 'gh api repos/Naartti/hEARo/issues/1/comments -fbody=x'
expect ask 'gh api repos/Naartti/hEARo/issues/1/comments -XPOST'
expect ask 'cd ../other-clone && gh pr comment 1 --body x'
expect ask 'git -C ../other-clone status && gh pr list'
CWD=/tmp expect ask 'gh pr comment 1 --body x'
CWD="$root/docs" expect allow 'gh pr list'

# Checking out a PR runs its code in `just ci`: only Dependabot's or the owner's.
FAKE_PR_AUTHOR=app/dependabot expect allow 'gh pr checkout 3'
FAKE_PR_AUTHOR=Naartti expect allow 'gh pr checkout 4 && just ci'
FAKE_PR_AUTHOR=stranger expect ask 'gh pr checkout 9'
FAKE_PR_AUTHOR='' expect ask 'gh pr checkout 9'

# The commit gate reads the green-check fingerprint of the checkout the commit happens in (a
# worktree has its own target/ci/check.ok), not CLAUDE_PROJECT_DIR's (2026-11-15: a worktree
# agent's green commit was judged by the main checkout's tree and blocked); `git -C <dir>
# commit` and `cd <dir> && git commit` are gated too (`-C` used to slip past the pattern).
wt="$(mktemp -d)"
git -C "$wt" init -q
mkdir -p "$wt/scripts" "$wt/target/ci"
cp "$root/scripts/tree-fingerprint.sh" "$wt/scripts/"
printf 'target/\n' >"$wt/.gitignore"
"$wt/scripts/tree-fingerprint.sh" >"$wt/target/ci/check.ok"
CWD="$wt" expect allow 'git commit -m "feat: x"'
expect allow "git -C $wt commit -m x"
expect allow "cd $wt && git commit -m x"
echo change >"$wt/new.txt"
CWD="$wt" expect block 'git commit -m "feat: x"'
expect block "git -C $wt commit -m x"
expect block "git -C '$wt' commit -m x"
expect block "cd $wt && git commit -m x"
CWD="$wt" expect block 'git -C . commit -am x'
CWD="$wt" expect allow 'git commit-tree HEAD^{tree} -m x'
rm -rf "$wt"

# Existing rules still hold.
expect block 'git commit --no-verify -m x'
expect block 'git push --force origin main'

if [[ "$fail" != 0 ]]; then exit 1; fi
echo "hook tests passed"
