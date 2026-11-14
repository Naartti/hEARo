---
name: release
description: Steps for cutting a hEARo release - version bump, changelog, just release, signing/notarization checks, tagging. Use when preparing a release or DMG.
---

# Release

1. `just ci` green on a clean tree.
2. Bump `version` in `Cargo.toml` (workspace) and `ml/pyproject.toml`; update CHANGELOG.md.
3. `just release` → `dist/hEARo-<version>.dmg`. Signs with Developer ID + hardened runtime +
   timestamp, notarizes and staples when credentials exist (`xcrun notarytool` keychain
   profile); otherwise produces an unnotarized DMG and says so.
4. Verify: `codesign --verify --deep --strict`, `spctl --assess -vv` (notarized only), mount
   the DMG (volume "hEARo" with the app icon, app + Applications link + read-me). On this Mac
   install with `just install` (the one copy lives at `~/Applications/hEARo.app`; never also
   drag the DMG's copy to /Applications, docs/dev/SIGNING.md), then run the hardware checklist
   smoke items. Check **About hEARo** shows the new version without "· debug".
5. Commit `chore(release): v<version>`, tag `v<version>`. Never push tags without the human.
