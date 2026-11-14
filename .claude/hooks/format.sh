#!/usr/bin/env bash
# PostToolUse(Edit/Write): format the edited file by extension. Never fails the edit.
f="$(python3 "$(dirname "$0")/_input.py" tool_input.file_path)"
[[ -f "$f" ]] || exit 0
cd "$CLAUDE_PROJECT_DIR" || exit 0
eval "$(mise env -s bash 2>/dev/null)" 2>/dev/null || true
case "$f" in
  *.rs)    rustfmt --edition 2024 --config-path rustfmt.toml "$f" 2>/dev/null ;;
  *.py)    (cd ml && uv run --frozen ruff format -q "$f" 2>/dev/null) ;;
  *.swift) command -v swift-format >/dev/null && swift-format format -i "$f" 2>/dev/null ;;
  *.md)    markdownlint-cli2 --fix "$f" >/dev/null 2>&1 ;;
esac
exit 0
