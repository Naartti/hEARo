#!/usr/bin/env bash
# Build the menu bar app with SwiftPM (works with Command Line Tools alone), assemble the .app
# bundle (Info.plist, icon), sign it with the stable identity and install it to a fixed path
# (§3.5, §16.2). One identity for every build: "hEARo", bundle id io.github.naartti.hearo, so the System
# Audio Recording permission and the login item are shared (docs/dev/SIGNING.md).
#
# Usage: scripts/build-app.sh [debug|release]
#   debug   → ~/Applications/hEARo.app via scripts/install-app.sh (quits a running hEARo
#             first, reopens it afterwards)
#   release → dist/hEARo.app (packaged by scripts/package-dmg.sh, installed by `just install`)
# HEARO_APP_DEST=/some/dir/hEARo.app builds into that path instead and touches nothing else:
# for inspecting a bundle without replacing the installed app.
. "$(dirname "$0")/lib.sh"
config="${1:-debug}"
name="hEARo"  # = CFBundleName/CFBundleExecutable in app/Resources/Info.plist
dest="${HEARO_APP_DEST:-$ROOT/dist/$name.app}"  # release (debug installs via install-app.sh)

scripts/build-ffi.sh release
dev="$(xcode_dev_dir || true)"
(cd app && DEVELOPER_DIR="${dev:-$(xcode-select -p)}" swift build -c "$config" --product hEARoApp 2>&1 | grep -vE "^\[[0-9]+/[0-9]+\]" || true)
bin="$(cd app && swift build -c "$config" --show-bin-path)/hEARoApp"
test -x "$bin"

# App icon: rendered from the SVG sources (cheap, ~2 s), so no binary lives in git.
icon_src=(app/Resources/Icon/hEARo.svg app/Resources/Icon/hEARo-small.svg scripts/make-icon.swift)
icns="$ROOT/target/icon/$name.icns"
if [[ ! -f "$icns" ]] || [[ -n "$(find "${icon_src[@]}" -newer "$icns")" ]]; then
  mkdir -p "$(dirname "$icns")"
  swift scripts/make-icon.swift "${icon_src[0]}" "${icon_src[1]}" "$icns"
fi

stage="$ROOT/target/app/$config/$name.app"
rm -rf "$stage"
mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
cp "$bin" "$stage/Contents/MacOS/$name"
cp "$icns" "$stage/Contents/Resources/$name.icns"
plist="$stage/Contents/Info.plist"
cp app/Resources/Info.plist "$plist"
version="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
sha="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
if [[ "$sha" != unknown ]] && ! git diff --quiet HEAD -- 2>/dev/null; then sha="$sha-dirty"; fi
plutil -replace CFBundleShortVersionString -string "$version" "$plist"
plutil -replace CFBundleVersion -string "$(git rev-list --count HEAD 2>/dev/null || echo 0)" "$plist"
plutil -replace HearoGitSHA -string "$sha" "$plist"
plutil -replace HearoBuildConfiguration -string "$config" "$plist"
plutil -replace HearoBuildDate -string "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$plist"
plutil -lint "$plist" >/dev/null
printf 'APPL????' > "$stage/Contents/PkgInfo"

scripts/sign.sh "$stage" $([[ "$config" == release ]] && echo --release)

if [[ -n "${HEARO_APP_DEST:-}" || "$config" == release ]]; then
  mkdir -p "$(dirname "$dest")"
  rm -rf "$dest"
  ditto "$stage" "$dest"
  echo "✔ built $dest"
else
  scripts/install-app.sh "$stage"
fi
