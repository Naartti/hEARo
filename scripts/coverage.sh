#!/usr/bin/env bash
# `just coverage` — Rust line coverage of the fast test suite (cargo-llvm-cov + nextest).
# Writes target/coverage/lcov.info (uploaded to Codecov by .github/workflows/ci.yml),
# target/coverage/summary.txt and an HTML report in target/coverage/html/.
# Hardware tests are #[ignore]d, so engine code that needs real Core Audio shows as uncovered.
. "$(dirname "$0")/lib.sh"
# llvm-tools ship with the pinned toolchain; installed on demand so bootstrap stays lean.
if ! rustup component list --installed 2>/dev/null | grep -q '^llvm-tools'; then
  rustup component add llvm-tools-preview
fi
out="target/coverage"
mkdir -p "$out"
# Generated UniFFI glue and the bindgen shim aren't ours to cover.
ignore='(hearo-bindgen/|/generated/)'
cargo llvm-cov clean --workspace
cargo llvm-cov nextest --workspace --no-tests=pass --ignore-filename-regex "$ignore" \
  --status-level fail --final-status-level fail --no-report
cargo llvm-cov report --ignore-filename-regex "$ignore" --lcov --output-path "$out/lcov.info"
cargo llvm-cov report --ignore-filename-regex "$ignore" --html --output-dir "$out"
cargo llvm-cov report --ignore-filename-regex "$ignore" --summary-only | tee "$out/summary.txt"
echo "✔ $out/lcov.info · HTML: $out/html/index.html"
