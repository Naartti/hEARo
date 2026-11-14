#!/usr/bin/env bash
# `just doctor` — check the dev environment and print how to fix anything missing.
. "$(dirname "$0")/lib.sh"
ok()   { printf '%s✔%s %s\n' "$GREEN" "$RESET" "$*"; }
bad()  { printf '%s✘%s %s\n   %s→ %s%s\n' "$RED" "$RESET" "$1" "$DIM" "$2" "$RESET"; FAILED_STEPS+=("$1"); }
soft() { printf '%s⚠%s %s\n   %s→ %s%s\n' "$YELLOW" "$RESET" "$1" "$DIM" "$2" "$RESET"; }

[[ "$(uname -m)" == arm64 ]] && ok "Apple Silicon" || bad "Not Apple Silicon" "hEARo supports Apple Silicon only"
ver="$(sw_vers -productVersion)"
[[ "${ver%%.*}" -ge 26 ]] && ok "macOS $ver" || bad "macOS $ver" "macOS 26 or later is required"

if missing="$(mise ls --current --missing 2>/dev/null)" && [[ -z "$missing" ]]; then
  ok "mise toolchains installed (.mise.toml)"
else
  bad "mise toolchains missing" "run: mise install"
fi
for t in cargo rustc just cargo-nextest cargo-deny cargo-machete uv lefthook xcodegen lychee markdownlint-cli2; do
  command -v "$t" >/dev/null && ok "$t $("$t" --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -1)" \
    || bad "$t missing" "run: mise install"
done
for t in xcbeautify; do
  command -v "$t" >/dev/null && ok "$t" || bad "$t missing" "run: brew bundle"
done
command -v swift >/dev/null && ok "swift $(swift --version 2>&1 | grep -oE 'Swift version [0-9.]+' | cut -d' ' -f3)" \
  || bad "swift missing" "install Xcode or run: xcode-select --install"

if dev="$(xcode_dev_dir)"; then
  ok "Xcode at $dev"
  [[ "$(xcode-select -p)" == "$dev" ]] || soft "xcode-select points at $(xcode-select -p)" \
    "optional: sudo xcode-select -s $dev (scripts already use DEVELOPER_DIR)"
  DEVELOPER_DIR="$dev" xcrun xcodebuild -checkFirstLaunchStatus >/dev/null 2>&1 \
    || soft "Xcode first-launch tasks pending" "run: sudo xcodebuild -runFirstLaunch (accepts license)"
else
  soft "Xcode not installed (Command Line Tools only)" \
    "Swift tests, previews and Instruments need Xcode (App Store). Everything else works."
fi

if [[ -f .env.local ]] && grep -q '^SIGN_IDENTITY=' .env.local; then
  id="$(grep '^SIGN_IDENTITY=' .env.local | cut -d= -f2- | tr -d '"')"
  if security find-identity -v -p codesigning | grep -qF "$id"; then ok "signing identity: $id"
  else bad "signing identity '$id' not in keychain" "see docs/dev/SIGNING.md"; fi
else
  soft "no SIGN_IDENTITY in .env.local" "needed for \`just app\` to keep audio permission across rebuilds; see docs/dev/SIGNING.md (H1)"
fi

[[ -f .git/hooks/pre-commit ]] && grep -q lefthook .git/hooks/pre-commit && ok "git hooks (lefthook)" \
  || bad "git hooks not installed" "run: lefthook install"
[[ -d ml/.venv ]] && ok "ml/.venv" || bad "ml venv missing" "run: (cd ml && uv sync)"

finish "just doctor"
