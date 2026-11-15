# hEARo — every developer and agent command. Run `just` to list.
# `just check` and `just ci` are the CI; GitHub Actions runs the same `just ci` (docs/adr/0016-github-actions-ci-and-releases.md).

# Recipes run through `mise x` so the pinned toolchains (.mise.toml) are used even in shells
# where mise isn't activated.
set shell := ["mise", "x", "--", "bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

# List recipes
default:
    @just --list --unsorted

# ── Setup ────────────────────────────────────────────────────────────────────

# Fresh clone → working environment (mise, brew, uv, git hooks), then doctor
bootstrap:
    scripts/bootstrap.sh

# Check tools, versions, Xcode, signing identity; prints fixes
doctor:
    scripts/doctor.sh

# ── Gates ────────────────────────────────────────────────────────────────────

# FAST GATE: fmt, clippy, all fast tests, swift build/test, ruff+pytest. Before every commit.
check:
    scripts/check.sh

# FULL LOCAL CI: check + golden + budgets + deny + machete + docs + xcodegen. Before push / milestone end.
ci:
    scripts/ci.sh

# Format everything (Rust, Python)
fmt:
    cargo fmt --all
    cd ml && uv run --frozen ruff format .

# Clippy with warnings as errors
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# ── Tests ────────────────────────────────────────────────────────────────────

# All tests except hardware and slow ML tests
test:
    cargo nextest run --workspace --no-tests=pass
    cargo test --workspace --doc --quiet
    scripts/swift-check.sh
    cd ml && uv run --frozen pytest -q -m "not slow"

# Offline golden audio tests (test binaries named golden*)
test-golden *ARGS:
    scripts/golden.sh {{ARGS}}

# Regenerate golden snapshots after an INTENTIONAL change (explain why in the commit message)
golden-update:
    INSTA_UPDATE=always scripts/golden.sh || true
    cargo insta review || true
    @echo "Commit with a message explaining why the golden metrics changed."

# Hardware tests (needs a human, audio permission and devices) — docs/testing/HARDWARE_CHECKLIST.md
# Hardware tests: switch the default output and back; watchdog with a tone playing (quit hEARo first). Checklist: docs/testing/HARDWARE_CHECKLIST.md
test-hw:
    cargo test -p hearo-ffi --test hw_output_switch --test hw_watchdog -- --ignored --nocapture --test-threads=1

# Rust line coverage (cargo-llvm-cov) → target/coverage/{lcov.info,summary.txt,html/}
coverage:
    scripts/coverage.sh

# Codecov's patch check, locally: coverage of the lines this branch adds (needs `just coverage`)
patch-coverage: coverage
    python3 scripts/patch_coverage.py

# Benchmarks (divan) + enforced CPU budgets (release build)
bench:
    cargo bench -p hearo-dsp --bench chain
    cargo test --release -p hearo-dsp --test budgets -- --ignored --nocapture

# ── Run ──────────────────────────────────────────────────────────────────────

# Run the hearo CLI (release build), e.g. `just cli process in.wav out.wav --modules gain --report`
cli *ARGS:
    cargo run --quiet --release -p hearo-cli -- {{ARGS}}

# Generate the reference test signals into scratch/signals
signals:
    cargo run --quiet --release -p hearo-cli -- gen-signals scratch/signals

# Build the Rust engine XCFramework + UniFFI Swift bindings into app/RustBridge (generated)
ffi:
    scripts/build-ffi.sh

# Build (debug), sign (stable identity) and install ~/Applications/hEARo.app (restarts it if running)
app:
    scripts/build-app.sh debug

# `just app`, then launch and stream logs
run: app
    open "$HOME/Applications/hEARo.app"
    log stream --style compact --predicate 'subsystem == "dev.hearo"'

# Render the app icon (app/Resources/Icon/*.svg) → target/icon/hEARo.icns + .iconset PNGs
icon:
    mkdir -p target/icon
    swift scripts/make-icon.swift app/Resources/Icon/hEARo.svg app/Resources/Icon/hEARo-small.svg target/icon/hEARo.icns

# Generate the Xcode project (git-ignored) and open it — optional, needs Xcode
xcode:
    xcodegen generate --spec app/project.yml --project app --project-root app
    open app/hEARo.xcodeproj

# ── Release ──────────────────────────────────────────────────────────────────

# Release build (bundle id dev.hearo.app) → sign → DMG in dist/ (unnotarized; docs/user/INSTALL.md)
release:
    scripts/build-app.sh release
    scripts/package-dmg.sh

# Install dist/hEARo.app as this Mac's only copy, ~/Applications/hEARo.app (replaces `just app`'s build)
install:
    scripts/install-app.sh dist/hEARo.app --open

# Quit and remove hEARo, its login item and (after asking) its data (M9)
uninstall:
    @echo "Arrives in M9."

# ── Docs & deps ──────────────────────────────────────────────────────────────

# Markdown lint + offline link check
docs:
    scripts/docs-check.sh

# License/advisory/source policy
deny:
    cargo deny check

# ── ML (M7) ──────────────────────────────────────────────────────────────────

# Python lint + fast tests
ml-check:
    scripts/ml-check.sh

# Download/prepare datasets (M7)
ml-data:
    @echo "Arrives in M7."

# Train a model (M7)
ml-train:
    @echo "Arrives in M7."

# Evaluate and write a report (M7)
ml-eval:
    @echo "Arrives in M7."

# Export ONNX (M7)
ml-export:
    @echo "Arrives in M7."
