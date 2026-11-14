#!/usr/bin/env bash
# `just bootstrap` — fresh clone → working dev environment. Idempotent.
. "$(dirname "$0")/lib.sh"
command -v mise >/dev/null || { echo "Install mise first: brew install mise"; exit 1; }
command -v brew >/dev/null || { echo "Install Homebrew first: https://brew.sh"; exit 1; }
mise trust --quiet "$ROOT"
mise install
eval "$(mise env -s bash)"
rustup target add aarch64-apple-darwin >/dev/null 2>&1 || true
brew bundle --file="$ROOT/Brewfile"
(cd ml && uv sync --frozen)
lefthook install
exec scripts/doctor.sh
