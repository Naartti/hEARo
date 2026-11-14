#!/usr/bin/env bash
# Build the Swift app package and run its tests.
# With only Command Line Tools, Swift Testing is unavailable: build only, warn about tests.
. "$(dirname "$0")/lib.sh"
scripts/build-ffi.sh   # the Swift package links the Rust engine (HearoLive)
cd app
if dev="$(xcode_dev_dir)"; then
  DEVELOPER_DIR="$dev" swift build --quiet
  DEVELOPER_DIR="$dev" swift test --quiet
else
  swift build --quiet
  echo "${YELLOW}⚠ Xcode not installed: Swift tests skipped (build only). See docs/OPEN_QUESTIONS.md (H1).${RESET}"
fi
