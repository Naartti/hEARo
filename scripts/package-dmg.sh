#!/usr/bin/env bash
# Package dist/hEARo.app into dist/hEARo-<version>.dmg (drag-to-Applications layout).
# Not notarized unless credentials exist (scripts/notarize.sh, H5); see docs/user/INSTALL.md.
# Usage: scripts/package-dmg.sh [App.app] [out.dmg]   (defaults: dist/hEARo.app,
# dist/hEARo-<version>-<sha>.dmg; the release workflow names it hEARo-<version>.dmg)
. "$(dirname "$0")/lib.sh"
app="${1:-dist/hEARo.app}"
[[ -d "$app" ]] || { echo "✘ $app missing — run scripts/build-app.sh release first"; exit 1; }
version="$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")"
sha="$(plutil -extract HearoGitSHA raw "$app/Contents/Info.plist")"
dmg="${2:-dist/hEARo-${version}-${sha}.dmg}"
mkdir -p "$(dirname "$dmg")"
rm -f "$dmg"
stage="$(mktemp -d)"
trap 'rm -rf "$stage" "$stage.rw.dmg"' EXIT
ditto "$app" "$stage/hEARo.app"
ln -s /Applications "$stage/Applications"
cp docs/user/INSTALL.md "$stage/Read me first.md" 2>/dev/null || true
cp "$app/Contents/Resources/hEARo.icns" "$stage/.VolumeIcon.icns"
# Plain hdiutil: create-dmg's Finder styling needs an Automation permission prompt (unattended
# builds would hang), and a styled window adds nothing for a personal install. The only extra is
# the volume icon (the app icon), which needs a writable image: build read-write, mount it
# hidden, set the custom-icon flag (uses .VolumeIcon.icns), then compress.
rw="$stage.rw.dmg"
hdiutil create -quiet -volname "hEARo" -srcfolder "$stage" -fs HFS+ -format UDRW "$rw"
mnt="$(mktemp -d)"
hdiutil attach -quiet -nobrowse -noautoopen -mountpoint "$mnt" "$rw"
SetFile -a C "$mnt"
hdiutil detach -quiet "$mnt"
hdiutil convert -quiet "$rw" -format UDZO -o "$dmg"
rm -f "$rw"
# Sign the disk image too, so its origin is verifiable.
# (An ad-hoc "-" identity adds nothing to a disk image, so skip it.)
load_sign_identity
if [[ -n "${SIGN_IDENTITY:-}" && "$SIGN_IDENTITY" != - ]]; then codesign --force --sign "$SIGN_IDENTITY" "$dmg"; fi
hdiutil verify "$dmg" >/dev/null
echo "✔ $dmg ($(du -h "$dmg" | cut -f1))"
