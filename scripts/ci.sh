#!/usr/bin/env bash
# `just ci` — the full CI gate. Runs locally (pre-push hook, milestone end, dependency/toolchain
# bumps) and unchanged in GitHub Actions (.github/workflows/ci.yml, docs/adr/0016-github-actions-ci-and-releases.md).
# Output is also written to target/ci/last.log.
. "$(dirname "$0")/lib.sh"
mkdir -p target/ci
exec > >(tee target/ci/last.log) 2>&1
echo "hEARo local CI — $(date '+%Y-%m-%d %H:%M:%S') — $(git rev-parse --short HEAD 2>/dev/null || echo 'no commit')"
step "rustfmt"            cargo fmt --all --check
step "clippy"             cargo clippy --workspace --all-targets --quiet -- -D warnings
step "rust tests"         cargo nextest run --workspace --no-tests=pass --status-level fail --final-status-level fail
step "golden tests"       scripts/golden.sh
step "doc tests"          cargo test --workspace --doc --quiet
step "benches compile"    cargo bench --workspace --no-run --quiet
# Timing on shared hosted runners is noise; budgets are enforced on a real Mac (ADR 0016).
if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
  warn "cpu budgets skipped on a hosted runner (run \`just bench\` on a Mac)"
else
  step "cpu budgets"      cargo test --release -p hearo-dsp --test budgets --quiet -- --ignored
fi
step "cargo-deny"         cargo deny --log-level error check
step "cargo-machete"      cargo machete
step "swift"              scripts/swift-check.sh
step "xcodegen spec"      scripts/xcodegen-validate.sh
step "python (ml)"        scripts/ml-check.sh
step "docs"               scripts/docs-check.sh
step "repo hygiene"       scripts/check-files.sh --all
finish "just ci"
stamp_green
