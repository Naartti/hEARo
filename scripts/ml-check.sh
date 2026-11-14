#!/usr/bin/env bash
# Lint and fast-test the Python ML package.
. "$(dirname "$0")/lib.sh"
cd ml
uv run --frozen ruff format --check .
uv run --frozen ruff check .
uv run --frozen pytest -q -m "not slow"
