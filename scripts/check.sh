#!/usr/bin/env bash
# `just check` — the fast gate. Must pass before EVERY commit (target: < 60 s warm).
. "$(dirname "$0")/lib.sh"
step "rustfmt"        cargo fmt --all --check
step "clippy"         cargo clippy --workspace --all-targets --quiet -- -D warnings
step "rust tests"     cargo nextest run --workspace --no-tests=pass --status-level fail --final-status-level fail
step "swift"          scripts/swift-check.sh
step "python (ml)"    scripts/ml-check.sh
step "repo hygiene"   scripts/check-files.sh
finish "just check"
stamp_green
