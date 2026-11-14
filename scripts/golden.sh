#!/usr/bin/env bash
# Offline golden audio tests: every test binary named golden_* (crates/*/tests/golden_*.rs).
. "$(dirname "$0")/lib.sh"
shopt -s nullglob
files=(crates/*/tests/golden_*.rs)
if ((${#files[@]} == 0)); then
  echo "no golden tests yet (they arrive with the M4 modules)"
  exit 0
fi
cargo nextest run --workspace -E 'binary(/^golden_/)' --status-level fail --final-status-level fail "$@"
