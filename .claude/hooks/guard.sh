#!/usr/bin/env bash
# PreToolUse(Edit/Write): block edits to generated files, golden data and datasets.
# Exit 2 = block, stderr is shown to Claude.
set -euo pipefail
f="$(python3 "$(dirname "$0")/_input.py" tool_input.file_path)"
rel="${f#"$CLAUDE_PROJECT_DIR"/}"
case "$rel" in
  *.xcodeproj/*|app/RustBridge/generated/*)
    echo "Blocked: $rel is generated. Change app/project.yml or the generator instead." >&2; exit 2 ;;
  ml/data/raw/*|ml/data/processed/*)
    echo "Blocked: datasets are produced by scripts (just ml-data), never edited by hand." >&2; exit 2 ;;
  testdata/*)
    if [[ "${HEARO_GOLDEN_UPDATE:-0}" != 1 ]]; then
      echo "Blocked: testdata/ holds golden files. Regenerate with 'just golden-update' and explain why in the commit." >&2; exit 2
    fi ;;
esac
exit 0
