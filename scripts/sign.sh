#!/usr/bin/env bash
# Sign an .app with the stable identity from .env.local (SIGN_IDENTITY).
# Ad-hoc signing loses the System Audio Recording permission on every rebuild (§3.5), so it is
# refused unless ALLOW_ADHOC=1. Usage: scripts/sign.sh path/to/App.app [--release]
. "$(dirname "$0")/lib.sh"
app="${1:?usage: sign.sh App.app [--release]}"
release="${2:-}"
[[ -f .env.local ]] && set -a && . ./.env.local && set +a
identity="${SIGN_IDENTITY:-}"
if [[ -z "$identity" || "$identity" == "-" ]]; then
  if [[ "${ALLOW_ADHOC:-0}" != 1 ]]; then
    echo "✘ No SIGN_IDENTITY in .env.local. Ad-hoc signing breaks audio permission on every rebuild."
    echo "  Set one up (docs/dev/SIGNING.md) or force ad-hoc with ALLOW_ADHOC=1."
    exit 1
  fi
  identity="-"
  echo "⚠ ad-hoc signing (ALLOW_ADHOC=1): permission will be lost on the next rebuild"
fi
ent="app/Resources/hEARo.entitlements"
opts=(--force --options runtime --entitlements "$ent" --sign "$identity")
[[ "$release" == --release ]] && opts+=(--timestamp)
# Nested code first (frameworks, dylibs), then the app itself.
while IFS= read -r -d '' nested; do
  codesign "${opts[@]}" "$nested"
done < <(find "$app/Contents" \( -name '*.framework' -o -name '*.dylib' \) -print0 2>/dev/null)
codesign "${opts[@]}" "$app"
codesign --verify --deep --strict "$app"
# All Mach-O files must share one Team ID.
teams="$(find "$app" -type f -perm -u+x -exec codesign -dv {} \; 2>&1 | grep -E '^TeamIdentifier=' | sort -u | wc -l | tr -d ' ')"
if (( teams > 1 )); then echo "✘ mixed Team IDs inside $app"; exit 1; fi
echo "✔ signed $app with '$identity'"
