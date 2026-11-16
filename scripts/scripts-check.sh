#!/usr/bin/env bash
# Lint and test the repo's Python scripts, then check changelog/progress fragments and that a
# feature branch leaves CHANGELOG.md and docs/PROGRESS.md alone (docs/dev/RELEASING.md).
. "$(dirname "$0")/lib.sh"
cd "$(dirname "$0")/.."
py=(scripts/changelog.py scripts/tests)
(cd ml && uv run --frozen ruff format --config pyproject.toml --check "${py[@]/#/../}")
(cd ml && uv run --frozen ruff check --config pyproject.toml "${py[@]/#/../}")
(cd ml && uv run --frozen pytest -q -p no:cacheprovider ../scripts/tests)
python3 scripts/changelog.py check
