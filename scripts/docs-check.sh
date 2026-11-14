#!/usr/bin/env bash
# Markdown lint + offline link check (local files and anchors only; we never hit the network).
. "$(dirname "$0")/lib.sh"
markdownlint-cli2 "**/*.md" "#target" "#.claude/worktrees" "#dist" "#**/node_modules" "#ml/.venv" "#app/.build"
lychee --offline --no-progress --include-fragments --exclude-path target --exclude-path ml/.venv --exclude-path app/.build --exclude-path .claude/worktrees --exclude-path dist '**/*.md'
